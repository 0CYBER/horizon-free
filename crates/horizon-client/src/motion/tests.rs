/// The glide's collision damage: a third of a block a tick lost is 0.0 and
/// does nothing, more than that hurts, and a move a block stopped loses
/// everything it had.
#[test]
fn a_glide_hurts_by_what_a_collision_took_off_its_speed() {
    assert_eq!(fly_into_wall_damage(0.5, 0.5), -3.0);
    assert_eq!(fly_into_wall_damage(0.3, 0.0), 0.0);
    assert!((fly_into_wall_damage(0.5, 0.1) - 1.0).abs() < 1e-5);
    assert!((fly_into_wall_damage(1.2, 0.0) - 9.0).abs() < 1e-5);
    assert!(fly_into_wall_damage(0.01, 0.0) < 0.0);
}

use super::*;

/// `LocalPlayer.isHorizontalCollisionMinor`: sliding along a wall at a
/// small angle to the input is minor; a sideways slide is not.
#[test]
fn horizontal_collision_minor_by_angle() {
    // Yaw 0 looks along +z; forward input.
    let fwd = [0.0, 1.0];
    assert!(horizontal_collision_minor(
        0.0,
        fwd,
        Vec3::new(0.02, 0.0, 0.2)
    ));
    assert!(!horizontal_collision_minor(
        0.0,
        fwd,
        Vec3::new(0.2, 0.0, 0.05)
    ));
    assert!(!horizontal_collision_minor(
        0.0,
        fwd,
        Vec3::new(0.0, 0.0, 0.0)
    ));
    assert!(!horizontal_collision_minor(
        0.0,
        [0.0, 0.0],
        Vec3::new(0.0, 0.0, 0.2)
    ));
}

#[test]
#[ignore = "daily"]
fn a_glide_that_just_ended_moves_slowly_until_the_pose_changes() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        eprintln!("skipped glide exit regression: prepared resources unavailable");
        return;
    };
    let resources = &v.motion;
    let mut p = PlayerMotion::new(resources, Vec3::ZERO).unwrap();
    p.set_pose(crate::pose::Pose::FallFlying).unwrap();
    // Gliding: LivingEntity.isVisuallySwimming is false.
    p.fall_flying = true;
    assert!(!p.moving_slowly());
    // The glide ended, the pose not yet: visually crawling.
    p.fall_flying = false;
    assert!(p.moving_slowly());
    // Not in water (Entity.isVisuallyCrawling).
    p.fluids.was_touching_water = true;
    assert!(!p.moving_slowly());
    p.fluids.was_touching_water = false;
    p.set_pose(crate::pose::Pose::Standing).unwrap();
    assert!(!p.moving_slowly());
}

#[test]
#[ignore = "daily"]
fn sword_sweep_requires_charge_ground_and_slow_horizontal_movement() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        eprintln!("skipped sword sweep regression: prepared resources unavailable");
        return;
    };
    let resources = &v.motion;
    let mut p = PlayerMotion::new(resources, Vec3::ZERO).unwrap();
    p.ground.on_ground = true;
    p.speed = 0.1;
    assert!(p.is_sweep_attack(1.0, false, true));
    assert!(!p.is_sweep_attack(0.9, false, true));
    assert!(!p.is_sweep_attack(1.0, true, true));
    assert!(!p.is_sweep_attack(1.0, false, false));
    p.sprinting = true;
    assert!(!p.is_sweep_attack(1.0, false, true));
    p.sprinting = false;
    p.ground.on_ground = false;
    assert!(!p.is_sweep_attack(1.0, false, true));
    p.ground.on_ground = true;
    p.velocity = Vec3::new(0.2, 10.0, 0.0);
    assert!(p.is_sweep_attack(1.0, false, true));
    p.velocity.z = 0.2;
    assert!(!p.is_sweep_attack(1.0, false, true));
    p.velocity = Vec3::new(f64::from(p.speed) * 2.5, 0.0, 0.0);
    assert!(!p.is_sweep_attack(1.0, false, true));
}

#[test]
#[ignore = "daily"]
fn max_health_modifier_changes_clamp_health() {
    // #8573: LivingEntity.onAttributeUpdated clamps health to max_health
    // for every modifier change, with no explicit clamp call.
    let Some(v) = crate::test_fixtures::vanilla() else {
        eprintln!("skipped health clamp: prepared resources unavailable");
        return;
    };
    let resources = &v.motion;
    let mut p = PlayerMotion::new(resources, Vec3::ZERO).unwrap();
    let boost = "minecraft:effect.health_boost";
    // Health boost I: +4 max health, and the hearts are filled.
    p.set_modifier("minecraft:max_health", boost, 4.0, "add_value", true)
        .unwrap();
    p.set_health(24.0, false).unwrap();
    // An unrelated attribute change leaves the boosted hearts alone.
    p.set_modifier("minecraft:armor", "minecraft:x", 2.0, "add_value", true)
        .unwrap();
    assert_eq!(p.health.health(), 24.0);
    p.set_modifier("minecraft:max_health", boost, 0.0, "add_value", false)
        .unwrap();
    assert_eq!(p.attribute("minecraft:max_health").unwrap(), 20.0);
    assert_eq!(p.health.health(), 20.0);
    // Below the maximum a lost modifier never raises or lowers health.
    p.set_modifier(
        "minecraft:max_health",
        "minecraft:gear",
        6.0,
        "add_value",
        true,
    )
    .unwrap();
    p.set_health(7.0, false).unwrap();
    p.set_modifier(
        "minecraft:max_health",
        "minecraft:gear",
        0.0,
        "add_value",
        false,
    )
    .unwrap();
    assert_eq!(p.health.health(), 7.0);
    // An equipment-style multiplier is covered the same way.
    p.set_modifier(
        "minecraft:max_health",
        "minecraft:gear",
        0.5,
        "add_multiplied_base",
        true,
    )
    .unwrap();
    p.set_health(30.0, false).unwrap();
    p.set_modifier(
        "minecraft:max_health",
        "minecraft:gear",
        0.0,
        "add_multiplied_base",
        false,
    )
    .unwrap();
    assert_eq!(p.health.health(), 20.0);
    // A remove-all/re-add batch keeps boosted hearts (unrelated effect).
    p.set_modifier("minecraft:max_health", boost, 4.0, "add_value", true)
        .unwrap();
    p.set_health(24.0, false).unwrap();
    p.set_modifier_unclamped("minecraft:max_health", boost, 0.0, "add_value", false)
        .unwrap();
    p.set_modifier_unclamped("minecraft:max_health", boost, 4.0, "add_value", true)
        .unwrap();
    p.clamp_health_to_max().unwrap();
    assert_eq!(p.health.health(), 24.0);
}

#[test]
#[ignore = "daily"]
fn attribute_bases_survive_a_save_and_reload() {
    // #4425: LivingEntity.addAdditionalSaveData writes AttributeMap.pack
    // and readAdditionalSaveData applies it (before the health).
    let Some(v) = crate::test_fixtures::vanilla() else {
        eprintln!("skipped attribute save: prepared resources unavailable");
        return;
    };
    let resources = &v.motion;
    let mut p = PlayerMotion::new(resources, Vec3::ZERO).unwrap();
    let set_base = |p: &mut PlayerMotion, name: &str, v: f64| {
        let key = p.attributes.key(name).unwrap();
        let h = p.attributes.get_instance(Some(key)).unwrap().unwrap();
        p.attributes
            .edit(&h, |a, o| a.set_base_value(v, o))
            .unwrap();
    };
    set_base(&mut p, "minecraft:scale", 2.0);
    set_base(&mut p, "minecraft:max_health", 30.0);
    let key = p.attributes.key("minecraft:max_health").unwrap();
    let h = p.attributes.get_instance(Some(key)).unwrap().unwrap();
    let bonus = Modifier::new("test:bonus", 0.5, Operation::AddMultipliedBase).unwrap();
    p.attributes
        .edit(&h, |a, o| a.add_permanent(bonus, o))
        .unwrap();
    let mut level = crate::save::Level {
        health: 40.0,
        food: 20,
        saturation: 5.0,
        player_extra: Some(horizon_core::nbt::Tag::compound([(
            "attributes",
            p.saved_attributes(),
        )])),
        ..Default::default()
    };
    // Through the player compound as the file holds it.
    let tag = level.player_tag();
    level = crate::save::Level::default();
    level.read_player(&tag);
    level.player_extra = Some(tag);
    let mut back = PlayerMotion::new(resources, Vec3::ZERO).unwrap();
    let mut pending = true;
    back.load_saved_state_once(&mut pending, &level).unwrap();
    let base = |p: &mut PlayerMotion, name: &str| {
        let key = p.attributes.key(name).unwrap();
        p.attributes.value(Some(key), ValueKind::Base).unwrap()
    };
    assert_eq!(base(&mut back, "minecraft:scale"), 2.0);
    assert_eq!(base(&mut back, "minecraft:max_health"), 30.0);
    assert_eq!(back.attribute("minecraft:max_health").unwrap(), 45.0);
    assert_eq!(back.health.health(), 40.0);
}

#[test]
#[ignore = "daily"]
fn auto_jump_detects_collidable_entities() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
    );
    let Some(v) = crate::test_fixtures::vanilla() else {
        panic!("no prepared resources at {}", root.display());
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(10.0, 100.0, 10.0)).unwrap();
    player.auto_jump_enabled = true;
    player.ground.on_ground = true;
    player.move_vector = [0.0, 1.0];
    let entities = [Aabb::new(
        Vec3::new(9.5, 100.0, 10.5),
        Vec3::new(10.5, 100.8, 11.5),
    )];
    MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &entities,
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    }
    .update_auto_jump(0.0, 0.1)
    .unwrap();
    assert_eq!(player.auto_jump_time, 1);
}

#[test]
#[ignore = "daily"]
fn player_stands_walks_sneaks_and_jumps_on_ghast() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let entities = [Aabb::new(
        Vec3::new(10.0, 80.0, 10.0),
        Vec3::new(14.0, 84.0, 14.0),
    )];
    let mut player = PlayerMotion::new(resources, Vec3::new(12.0, 84.0, 12.0)).unwrap();
    let mut tick = |player: &mut PlayerMotion, keys| {
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &entities,
            player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .controlled_simulation_tick(keys)
        .unwrap()
    };
    for _ in 0..20 {
        tick(&mut player, Default::default());
    }
    assert_eq!(player.position.y, 84.0);
    assert!(player.ground.on_ground);
    let forward = crate::sprint::Keys {
        forward: true,
        ..Default::default()
    };
    for _ in 0..5 {
        tick(&mut player, forward);
    }
    assert!(player.position.z > 12.0);
    assert_eq!(player.position.y, 84.0);
    tick(
        &mut player,
        crate::sprint::Keys {
            jump: true,
            ..Default::default()
        },
    );
    assert!(player.position.y > 84.0);
    for _ in 0..30 {
        tick(&mut player, Default::default());
    }
    assert_eq!(player.position.y, 84.0);
    for _ in 0..80 {
        tick(
            &mut player,
            crate::sprint::Keys {
                shift: true,
                ..forward
            },
        );
    }
    assert_eq!(player.position.y, 84.0);
    assert!(player.position.z < 14.3);
    for _ in 0..30 {
        tick(&mut player, forward);
    }
    assert!(player.position.y < 84.0);
}

/// `MovingPistonBlock.getCollisionShape`: a block halfway through a push
/// up is solid at its moving position, so a player lands on it.
#[test]
#[ignore = "daily"]
fn a_player_lands_on_a_block_a_piston_is_moving() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let registry = assets.registry;
    let moving = registry.default_state("minecraft:moving_piston").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    scene.set(20, 4, 20, moving).unwrap();
    scene.moving_pistons.push((
        [20, 4, 20],
        horizon_world::piston_shape::MovingPistonState {
            moved_state: stone,
            direction: horizon_core::geometry::Direction::Up,
            extending: true,
            source: false,
            progress: 0.5,
            previous_progress: 0.0,
        },
    ));
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 6.0, 20.5)).unwrap();
    for _ in 0..30 {
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .controlled_simulation_tick(Default::default())
        .unwrap();
    }
    assert!(player.ground.on_ground);
    assert_eq!(player.position.y, 4.5);
    // Once the block entity is gone, the moving_piston block has no shape.
    scene.moving_pistons.clear();
    for _ in 0..10 {
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .controlled_simulation_tick(Default::default())
        .unwrap();
    }
    assert_eq!(player.position.y, 4.0);
}

#[test]
#[ignore = "daily"]
fn a_lava_cauldron_burns_the_player_inside_it() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let cauldron = assets
        .registry
        .default_state("minecraft:lava_cauldron")
        .unwrap();
    scene.set(20, 4, 20, cauldron).unwrap();
    let mut tick = |player: &mut PlayerMotion| {
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .controlled_simulation_tick(Default::default())
        .unwrap();
    };
    // Standing on the cauldron's floor, in its lava.
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.25, 20.5)).unwrap();
    player.ticks_frozen = 40;
    tick(&mut player);
    assert!(player.health.health() < 20.0, "the lava must hurt");
    assert!(player.fire_ticks > 0, "the lava must ignite");
    assert_eq!(player.ticks_frozen, 0);
    // Beside it, nothing.
    let mut away = PlayerMotion::new(resources, Vec3::new(22.5, 4.0, 20.5)).unwrap();
    tick(&mut away);
    assert_eq!(away.health.health(), 20.0);
    assert!(away.fire_ticks <= 0);
}

#[test]
#[ignore = "daily"]
fn a_water_cauldron_puts_out_a_burning_player() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let water = assets
        .registry
        .resolve(&horizon_world::chunk::BlockState {
            name: "minecraft:water_cauldron".into(),
            properties: [("level".to_owned(), "3".to_owned())].into(),
        })
        .unwrap();
    scene.set(20, 4, 20, water).unwrap();
    let mut tick = |player: &mut PlayerMotion| {
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .controlled_simulation_tick(Default::default())
        .unwrap();
    };
    let asks = |player: &PlayerMotion| {
        player
            .events
            .iter()
            .filter(|e| matches!(e, MotionEvent::CauldronExtinguish { .. }))
            .count()
    };
    // Burning, on the cauldron's floor in its water: put out, and the
    // server is asked to lower the water.
    let mut burning = PlayerMotion::new(resources, Vec3::new(20.5, 4.25, 20.5)).unwrap();
    burning.fire_ticks = 100;
    tick(&mut burning);
    assert!(burning.fire_ticks <= 0, "the water must put the fire out");
    assert_eq!(asks(&burning), 1);
    // Not burning: nothing to lower.
    let mut dry = PlayerMotion::new(resources, Vec3::new(20.5, 4.25, 20.5)).unwrap();
    tick(&mut dry);
    assert_eq!(asks(&dry), 0);
}

/// `PowderSnowBlock.entityInside`: a burning player in powder snow asks
/// for the block to be destroyed, and EXTINGUISH puts the fire out.
#[test]
#[ignore = "daily"]
fn a_burning_player_melts_powder_snow() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let snow = assets
        .registry
        .default_state("minecraft:powder_snow")
        .unwrap();
    scene.set(20, 4, 20, snow).unwrap();
    let mut run = |fire: i32| {
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        player.fire_ticks = fire;
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .controlled_simulation_tick(Default::default())
        .unwrap();
        player
    };
    let burning = run(100);
    assert!(burning.fire_ticks <= 0, "powder snow must put the fire out");
    assert!(
        burning
            .events
            .iter()
            .any(|e| matches!(e, MotionEvent::PowderSnowMelt { .. }))
    );
    let dry = run(-20);
    assert!(
        !dry.events
            .iter()
            .any(|e| matches!(e, MotionEvent::PowderSnowMelt { .. }))
    );
}

/// A server's player lands in a cobweb or a sweet berry bush after a long
/// fall (#12382): `handleMovePlayer` moves it first, and the block's
/// `makeStuckInBlock` forgets the fall before `doCheckFallDamage`, so it
/// is not hurt. The same fall onto a block with nothing in it is hurt for
/// floor(10 + 1.5 + 1e-6 - 3) = 8 (`LivingEntity.calculateFallDamage`); a
/// flying player (whose own client never lands) is still hurt by a fall it
/// carries, as `Player.makeStuckInBlock` ignores it.
#[test]
#[ignore = "daily"]
fn a_server_player_landing_in_a_web_or_bush_takes_no_fall_damage() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let web = assets.registry.default_state("minecraft:cobweb").unwrap();
    let bush = assets
        .registry
        .default_state("minecraft:sweet_berry_bush")
        .unwrap();
    scene.set(20, 4, 20, web).unwrap();
    scene.set(24, 4, 20, bush).unwrap();
    let land = |scene: &mut World, x: f64, flying: bool| {
        let at = Vec3::new(x, 4.0, 20.5);
        let mut player = PlayerMotion::new(resources, at).unwrap();
        player.fall_distance = 10.0;
        player.flying = flying;
        let from = at.add_vector(Vec3::new(0.0, 1.5, 0.0));
        let delta = at.subtract(from);
        player
            .history
            .add(horizon_world::entity_move::MovementRecord {
                from,
                to: at,
                original: Some(delta),
            });
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        // The order of the server's tick: the early pass, then the landing.
        w.server_stuck_in_blocks(from).unwrap();
        let pos = BlockPos::new(x.floor() as i32, 3, 20);
        let state = w.block(pos);
        w.check_fall_damage(delta.y, true, state, pos).unwrap();
        // The moves are still there for server_block_damage.
        assert_eq!(w.player.history.records().len(), 1);
        player
    };
    assert_eq!(land(&mut scene, 20.5, false).health.health(), 20.0);
    assert_eq!(land(&mut scene, 24.5, false).health.health(), 20.0);
    assert_eq!(land(&mut scene, 22.5, false).health.health(), 12.0);
    assert_eq!(land(&mut scene, 20.5, true).health.health(), 12.0);
}

/// A server's player sinking in powder snow: `PowderSnowBlock.entityInside`
/// calls `makeStuckInBlock` while the block at its feet is powder snow,
/// which forgets the fall in the same early pass, so the fall that carried
/// it into the column is not counted (`fall_one_cm`) and the stone under
/// the column does not hurt. The same sink with no snow in it is hurt.
#[test]
#[ignore = "daily"]
fn a_server_player_sinking_in_powder_snow_takes_no_fall_damage() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let snow = assets
        .registry
        .default_state("minecraft:powder_snow")
        .unwrap();
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    scene.set(20, 2, 20, stone).unwrap();
    scene.set(20, 3, 20, snow).unwrap();
    scene.set(20, 4, 20, snow).unwrap();
    scene.set(24, 2, 20, stone).unwrap();
    // The feet end on the stone, inside the column's bottom block.
    let sink = |scene: &mut World, x: f64| {
        let at = Vec3::new(x, 3.0, 20.5);
        let mut player = PlayerMotion::new(resources, at).unwrap();
        player.fall_distance = 10.0;
        let from = Vec3::new(x, 4.5, 20.5);
        let delta = at.subtract(from);
        player
            .history
            .add(horizon_world::entity_move::MovementRecord {
                from,
                to: at,
                original: Some(delta),
            });
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        w.server_stuck_in_blocks(from).unwrap();
        let remembered = w.player.fall_distance;
        let pos = BlockPos::new(x.floor() as i32, 2, 20);
        let state = w.block(pos);
        w.check_fall_damage(delta.y, true, state, pos).unwrap();
        (player, remembered)
    };
    let (sunk, remembered) = sink(&mut scene, 20.5);
    assert_eq!(remembered, 0.0, "the column forgets the fall");
    assert_eq!(sunk.health.health(), 20.0, "no fall damage on the stone");
    assert!(
        !sunk.events.iter().any(|e| matches!(
            e,
            MotionEvent::Fall(
                crate::landing::FallEvent::Statistic(_) | crate::landing::FallEvent::Damage { .. }
            )
        )),
        "no fall distance is counted and no fall damage is dealt"
    );
    assert_eq!(sink(&mut scene, 24.5).0.health.health(), 12.0, "bare stone");
}

#[test]
#[ignore = "daily"]
fn standing_in_a_wither_rose_asks_for_its_effect() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let rose = assets
        .registry
        .default_state("minecraft:wither_rose")
        .unwrap();
    scene.set(20, 4, 20, rose).unwrap();
    let roses = |player: &PlayerMotion| {
        player
            .events
            .iter()
            .filter(|e| matches!(e, MotionEvent::InsideWitherRose))
            .count()
    };
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    }
    .controlled_simulation_tick(Default::default())
    .unwrap();
    assert_eq!(roses(&player), 1);
    // A server's player: its client moved it, the server applies the effect.
    player.events.clear();
    let from = player.position;
    MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    }
    .server_block_damage(from)
    .unwrap();
    assert_eq!(roses(&player), 1);
    // Two blocks away, nothing.
    let mut away = PlayerMotion::new(resources, Vec3::new(22.5, 4.0, 20.5)).unwrap();
    MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut away,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    }
    .controlled_simulation_tick(Default::default())
    .unwrap();
    assert_eq!(roses(&away), 0);
}

/// #8156: a window's own player carried by a boat does not travel, but it still freezes
/// in powder snow and thaws out of it (`LivingEntity.aiStep`), or the frostbite overlay,
/// which reads `ticks_frozen`, never shows.
#[test]
#[ignore = "daily"]
fn a_passenger_in_powder_snow_freezes_and_thaws() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let snow = assets
        .registry
        .default_state("minecraft:powder_snow")
        .unwrap();
    scene.set(20, 4, 20, snow).unwrap();
    let in_snow = Vec3::new(20.5, 4.0, 20.5);
    let clear = Vec3::new(30.5, 4.0, 30.5);
    let mut player = PlayerMotion::new(resources, in_snow).unwrap();
    player.client_side = true;
    let tick = |player: &mut PlayerMotion, scene: &mut World| {
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player,
            scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .passenger_block_effects_tick()
        .unwrap();
    };
    // One tick of frost for each tick in powder snow, up to the 140 that freeze fully.
    for expected in 1..=5 {
        tick(&mut player, &mut scene);
        assert!(player.in_powder_snow);
        assert_eq!(player.ticks_frozen, expected);
    }
    for _ in 0..200 {
        tick(&mut player, &mut scene);
    }
    assert_eq!(player.ticks_frozen, PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
    assert!(player.is_fully_frozen());
    // Carried out of it: two ticks of frost thaw each tick.
    player.position = clear;
    player.old_position = clear;
    player.ticks_frozen = 5;
    for expected in [3, 1, 0, 0] {
        tick(&mut player, &mut scene);
        assert!(!player.in_powder_snow);
        assert_eq!(player.ticks_frozen, expected);
    }
    // What cannot freeze (a freeze-immune player) takes no frost in it.
    player.position = in_snow;
    player.old_position = in_snow;
    player.freeze_immune = true;
    tick(&mut player, &mut scene);
    assert_eq!(player.ticks_frozen, 0);
    // Block effects off (a mod's switch): nothing is sampled, the frost thaws.
    player.freeze_immune = false;
    player.block_effects = false;
    player.ticks_frozen = 10;
    tick(&mut player, &mut scene);
    assert!(!player.in_powder_snow);
    assert_eq!(player.ticks_frozen, 8);
}

#[test]
#[ignore = "daily"]
fn walking_through_powder_snow_asks_for_a_snowflake() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let snow = assets
        .registry
        .default_state("minecraft:powder_snow")
        .unwrap();
    scene.set(20, 4, 20, snow).unwrap();
    let at = Vec3::new(20.5, 4.0, 20.5);
    let from = at.subtract(Vec3::new(0.2, 0.0, 0.0));
    let snowflakes = |player: &PlayerMotion| {
        player
            .events
            .iter()
            .filter(|e| matches!(e, MotionEvent::Snowflake { .. }))
            .count()
    };
    let mut player = PlayerMotion::new(resources, at).unwrap();
    let mut found = 0;
    for seed in 0..64 {
        scene.random.set_seed(seed);
        player.events.clear();
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .server_block_damage(from)
        .unwrap();
        found = snowflakes(&player);
        if found > 0 {
            break;
        }
    }
    assert!(
        found > 0,
        "a server's player walking through powder snow must send a snowflake"
    );
    if let MotionEvent::Snowflake { position, velocity } = &player
        .events
        .iter()
        .find(|e| matches!(e, MotionEvent::Snowflake { .. }))
        .unwrap()
    {
        assert_eq!(position.y, 5.0);
        assert_eq!(velocity.y, f64::from(0.05_f32));
    }
    player.events.clear();
    scene.random.set_seed(1);
    MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    }
    .server_block_damage(at)
    .unwrap();
    assert_eq!(snowflakes(&player), 0);
    player.client_side = true;
    player.events.clear();
    MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    }
    .controlled_simulation_tick(Default::default())
    .unwrap();
    assert_eq!(snowflakes(&player), 0);
}

#[test]
#[ignore = "daily"]
fn server_travel_moves_only_the_velocity() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    let at = Vec3::new(20.5, 4.0, 20.5);
    let mut player = PlayerMotion::new(resources, at).unwrap();
    let mut travel = |player: &mut PlayerMotion| {
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .server_travel()
        .unwrap()
    };
    // A knockback on the ground (vanilla's server sends 0.21840 0.27516
    // after a zombie's hit): ground friction, then gravity and drag.
    player.ground.on_ground = true;
    player.velocity = Vec3::new(-0.4, 0.3608, 0.0);
    travel(&mut player);
    assert!(
        (player.velocity.x + 0.2184).abs() < 1e-6,
        "{:?}",
        player.velocity
    );
    assert!(
        (player.velocity.y - 0.275184).abs() < 1e-6,
        "{:?}",
        player.velocity
    );
    assert_eq!(player.position, at);
    assert!(player.ground.on_ground);
    // Falling onto the ground hurts no one on the server's copy, and leaves
    // the fall to the client's moves.
    let health = player.health.health();
    player.position = Vec3::new(20.5, 4.5, 20.5);
    player.ground.on_ground = false;
    player.fall_distance = 10.0;
    player.velocity = Vec3::new(0.0, -1.0, 0.0);
    travel(&mut player);
    assert_eq!(player.velocity.y, -0.0784000015258789);
    assert_eq!(player.health.health(), health);
    assert_eq!(player.fall_distance, 10.0);
    assert!(player.events.is_empty());
}

/// `LivingEntity.handleFallFlyingCollisions`: a glide that meets a wall loses
/// the horizontal speed it had, and the server's copy hurts by that loss,
/// `(moveHorLength - newMoveHorLength) * 10 - 3`, as `minecraft:fly_into_wall`.
/// The speed it starts the tick with is the client's own movement
/// (`handleMovePlayer` gives the server copy its delta movement), so the
/// loss is the speed the glide actually flew at.
#[test]
#[ignore = "daily"]
fn a_glide_into_a_wall_hurts_by_the_speed_it_lost() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    // A wall in the way of a glide along +x, at feet height.
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    for y in 4..=5 {
        for z in 19..=21 {
            scene.set(23, y, z, stone).unwrap();
        }
    }
    let mut player = PlayerMotion::new(resources, Vec3::new(22.0, 4.0, 20.5)).unwrap();
    player.fall_flying = true;
    // Looking along +x (the `getLookAngle` of a flat -90 yaw) keeps the
    // glide on its course through `updateFallFlyingMovement`'s pull.
    player.yaw = -90.0;
    player.pitch = 0.0;
    player.carry_client_movement(Vec3::new(1.2, 0.0, 0.0));
    MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    }
    .server_travel()
    .unwrap();
    assert!(
        player
            .hurt_log
            .last()
            .is_some_and(|hurt| hurt.source == "minecraft:fly_into_wall"),
        "{:?}",
        player.hurt_log
    );
    assert!(player.health.health() < 20.0, "{}", player.health.health());
}

/// A client's own glide into a wall plays its fall sound but never hurts
/// it (`handleFallFlyingCollisions`' playSound runs on both sides, the
/// hurt is the server's alone), so the glider hears it.
#[test]
#[ignore = "daily"]
fn a_clients_own_glide_into_a_wall_plays_the_fall_sound_and_does_not_hurt_it() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 1).unwrap();
    // A wall in the way of a glide along +x, at feet height.
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    for y in 4..=5 {
        for z in 19..=21 {
            scene.set(23, y, z, stone).unwrap();
        }
    }
    let mut player = PlayerMotion::new(resources, Vec3::new(22.0, 4.0, 20.5)).unwrap();
    player.client_side = true;
    player.fall_flying = true;
    // Looking along +x keeps the glide on its course into the wall.
    player.yaw = -90.0;
    player.pitch = 0.0;
    player.carry_client_movement(Vec3::new(1.2, 0.0, 0.0));
    let loop_context =
        crate::test_resources::loop_context(horizon_world::native_load_context::LoopSide::Client);
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: &loop_context,
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    w.travel_fall_flying(&mut MoveGate::new()).unwrap();
    let is_fall = |e: &_| {
        matches!(
            e,
            MotionEvent::Fall(crate::landing::FallEvent::Sound { .. })
        )
    };
    assert_eq!(w.player.events.iter().filter(|e| is_fall(e)).count(), 1);
    // The sound must play only once on impact, not repeat on subsequent ticks (#12304):
    w.travel_fall_flying(&mut MoveGate::new()).unwrap();
    assert_eq!(w.player.events.iter().filter(|e| is_fall(e)).count(), 1);
    assert!(
        !w.player
            .hurt_log
            .last()
            .is_some_and(|hurt| hurt.source == "minecraft:fly_into_wall"),
        "a client's own glide does not hurt it: {:?}",
        w.player.hurt_log
    );
    assert_eq!(w.player.health.health(), 20.0);
}

#[test]
#[ignore = "daily"]
fn spectator_scroll_speed_controls_horizontal_and_vertical_flight() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let run = |steps| {
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        let start = Vec3::new(20.5, 12.0, 20.5);
        let mut player = PlayerMotion::new(resources, start).unwrap();
        player.spectator = true;
        player.flying_speed = crate::mouse::spectator_flying_speed(player.flying_speed, steps);
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .controlled_simulation_tick(crate::sprint::Keys {
            forward: true,
            jump: true,
            ..Default::default()
        })
        .unwrap();
        player.position.subtract(start)
    };
    let stopped = run(-100);
    assert_eq!(stopped, Vec3::ZERO);
    let normal = run(0);
    let fast = run(10);
    assert!(normal.y > 0.0 && normal.z > 0.0);
    assert!((fast.y - normal.y * 2.0).abs() < 1e-6);
    assert!((fast.z - normal.z * 2.0).abs() < 1e-6);
}

#[test]
#[ignore = "daily"]
fn resident_spectator_flight_ignores_solid_and_climbable_blocks() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let run = |name: &str, spectator: bool| {
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        let state = assets.registry.default_state(name).unwrap();
        for x in 19..=23 {
            for y in 10..=17 {
                for z in 19..=23 {
                    scene.set(x, y, z, state).unwrap();
                }
            }
        }
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 12.0, 20.5)).unwrap();
        player.spectator = spectator;
        player.flying = spectator;
        player.velocity = Vec3::new(0.4, 0.1, 0.4);
        let keys = crate::sprint::Keys {
            forward: true,
            jump: true,
            ..Default::default()
        };
        let mut trajectory = Vec::new();
        for _ in 0..4 {
            MotionWorld {
                tag_items: None,
                block_registries: None,
                loop_context: crate::test_resources::server_loop_context(),
                damage_hooks: &crate::health::DamageHooks::NONE,
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets,
                collision_binding: binding,
                resources,
                health: &resources.health,
            }
            .controlled_simulation_tick(keys)
            .unwrap();
            trajectory.push((player.position, player.velocity));
        }
        trajectory
    };
    let air = run("minecraft:air", true);
    for name in [
        "minecraft:stone",
        "minecraft:ladder",
        "minecraft:vine",
        "minecraft:scaffolding",
    ] {
        assert_eq!(run(name, true), air, "spectator flight in {name}");
    }
    let air = run("minecraft:air", false);
    let ladder = run("minecraft:ladder", false);
    assert!(air.last().unwrap().1.y < 0.0);
    assert!(ladder.last().unwrap().1.y > 0.0);
}

#[test]
#[ignore = "daily"]
fn creative_and_spectator_preserve_hunger_when_switching_from_survival() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let Some(v) = crate::test_fixtures::vanilla() else {
        eprintln!("skipped hunger regression: prepared resources unavailable");
        return;
    };
    let resources = &v.motion;
    // Both abilities are set by creative/spectator mode changes for the
    // resident player and for LAN/dedicated guests.
    for spectator in [false, true] {
        for food in [0, 1, 20] {
            let mut player = PlayerMotion::new(resources, Vec3::ZERO).unwrap();
            player.health.food.food = food;
            player.health.food.saturation = 0.0;
            player.health.food.exhaustion = 0.0;
            player.health.food.timer = 79;
            player.invulnerable = true;
            player.spectator = spectator;
            for _ in 0..160 {
                player.cause_food_exhaustion(0.2);
                player
                    .food_tick(
                        crate::test_resources::server_loop_context(),
                        level,
                        &resources.health,
                        Default::default(),
                    )
                    .unwrap();
            }
            assert_eq!(player.health.food.food, food);
            assert_eq!(player.health.food.saturation, 0.0);
            assert_eq!(player.health.food.exhaustion, 0.0);
            assert_eq!(player.health.health(), 20.0);
            assert!(!player.health.dead_or_dying());
            assert!(player.events.is_empty());

            // Pending exhaustion from survival is still processed, but an
            // empty food bar cannot hurt the now-invulnerable player.
            player.health.food.exhaustion = 5.0;
            player.health.food.timer = 79;
            player
                .food_tick(
                    crate::test_resources::server_loop_context(),
                    level,
                    &resources.health,
                    Default::default(),
                )
                .unwrap();
            assert_eq!(player.health.food.food, (food - 1).max(0));
            assert_eq!(player.health.food.exhaustion, 1.0);
            assert_eq!(player.health.health(), 20.0);
            assert!(player.events.is_empty());

            // Returning to survival resumes exhaustion and starvation.
            player.health.food.food = food;
            player.health.food.exhaustion = 0.0;
            player.invulnerable = false;
            player.spectator = false;
            player.cause_food_exhaustion(0.2);
            assert_eq!(player.health.food.exhaustion, 0.2);
            player.health.food.exhaustion = 5.0;
            player.health.food.timer = 79;
            player
                .food_tick(
                    crate::test_resources::server_loop_context(),
                    level,
                    &resources.health,
                    Default::default(),
                )
                .unwrap();
            assert_eq!(player.health.food.food, (food - 1).max(0));
            assert_eq!(player.health.health(), if food <= 1 { 19.0 } else { 20.0 });
            assert_eq!(
                player.events.iter().any(|event| matches!(
                    event,
                    MotionEvent::Health(crate::health::HealthEvent::StarvationDamage(1.0))
                )),
                food <= 1
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn player_fire_damage_queues_hurt_sound_only_for_fresh_hits() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let Some(v) = crate::test_fixtures::vanilla() else {
        eprintln!("skipped damage regression: prepared resources unavailable");
        return;
    };
    let resources = &v.motion;
    for name in ["on_fire", "in_fire", "lava"] {
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let source = resources
            .health
            .damage
            .id(&format!("minecraft:{name}"))
            .unwrap();
        assert!(
            player
                .hurt(level, &resources.health, Default::default(), source, 1.0)
                .unwrap()
        );
        assert_eq!(
            player
                .events
                .iter()
                .filter(|e| matches!(
                    e,
                    MotionEvent::HurtSound("minecraft:entity.player.hurt_on_fire")
                ))
                .count(),
            1
        );
        assert_eq!(player.health.hurt_animation_time, 10);
        player.health.advance_clock();
        player.events.clear();
        assert!(
            !player
                .hurt(level, &resources.health, Default::default(), source, 1.0)
                .unwrap()
        );
        assert!(
            !player
                .events
                .iter()
                .any(|e| matches!(e, MotionEvent::HurtSound(_)))
        );
        assert_eq!(
            player.health.hurt_animation_time, 9,
            "cooldown hit must not restart animation"
        );
        for _ in 0..20 {
            player.health.advance_clock();
        }
        assert_eq!(player.health.hurt_animation_time, 0);
        player.invulnerable = true;
        player.events.clear();
        assert!(
            !player
                .hurt(level, &resources.health, Default::default(), source, 1.0)
                .unwrap()
        );
        assert!(
            !player
                .events
                .iter()
                .any(|e| matches!(e, MotionEvent::HurtSound(_)))
        );
    }
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    let source = resources.health.damage.id("minecraft:mob_attack").unwrap();
    assert!(
        player
            .hurt_blocking(
                level,
                &resources.health,
                Default::default(),
                source,
                2.0,
                None
            )
            .unwrap()
            .hurt
    );
    assert_eq!(
        player
            .events
            .iter()
            .filter(|e| matches!(e, MotionEvent::HurtSound("minecraft:entity.player.hurt")))
            .count(),
        1
    );
    // A fatal hit plays the death sound rather than a hurt sound (#4570).
    let mut dying = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    assert!(
        dying
            .hurt(level, &resources.health, Default::default(), source, 100.0)
            .unwrap()
    );
    assert!(dying.health.dead_or_dying());
    assert!(
        !dying
            .events
            .iter()
            .any(|e| matches!(e, MotionEvent::HurtSound(_)))
    );
    // A shield that takes the whole arrow: onBlocked's sound, no damage
    // event, so the player hears no hurt sound.
    use horizon_world::blocking_rules::{BlockingRules, DamageReduction, ItemDamageFunction};
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    let source = resources.health.damage.id("minecraft:arrow").unwrap();
    let shield = crate::health::Blocking {
        rules: BlockingRules {
            delay_seconds: 0.25,
            disable_scale: 1.0,
            reductions: vec![DamageReduction {
                horizontal_angle: 90.0,
                types: None,
                base: 0.0,
                factor: 1.0,
            }],
            item_damage: ItemDamageFunction {
                threshold: 3.0,
                base: 1.0,
                factor: 1.0,
            },
        },
        bypassed: false,
        pierce: false,
        angle: 0.0,
    };
    let outcome = player
        .hurt_blocking(
            level,
            &resources.health,
            Default::default(),
            source,
            4.0,
            Some(shield.clone()),
        )
        .unwrap();
    assert!(!outcome.hurt && outcome.sound && outcome.blocked == 4.0);
    assert_eq!(
        player.health.hurt.hurt_time, 10,
        "preserve server damage bookkeeping"
    );
    assert_eq!(
        player.health.hurt_animation_time, 0,
        "a fully blocked arrow must not animate hurt"
    );
    assert!(
        !player
            .events
            .iter()
            .any(|e| matches!(e, MotionEvent::HurtSound(_)))
    );
    // An unblocked hit after the shield block must still animate normally.
    for _ in 0..20 {
        player.health.advance_clock();
    }
    assert!(
        player
            .hurt(level, &resources.health, Default::default(), source, 4.0)
            .unwrap()
    );
    assert_eq!(player.health.hurt_animation_time, 10);
    player.health.advance_clock();
    assert!(
        !player
            .hurt_blocking(
                level,
                &resources.health,
                Default::default(),
                source,
                4.0,
                Some(shield)
            )
            .unwrap()
            .hurt
    );
    assert_eq!(
        player.health.hurt_animation_time, 9,
        "blocking must not erase or restart an existing animation"
    );
    let mut starving = PlayerMotion::new(resources, Vec3::ZERO).unwrap();
    starving.health.food.food = 0;
    starving.health.food.timer = 79;
    starving
        .food_tick(
            crate::test_resources::server_loop_context(),
            level,
            &resources.health,
            Default::default(),
        )
        .unwrap();
    assert_eq!(starving.health.health(), 19.0);
    assert_eq!(
        starving.health.hurt_animation_time, 10,
        "starvation must still animate hurt"
    );
}

#[test]
#[ignore = "daily"]
fn a_glide_tick_matches_vanilla_to_the_bit() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    // LivingEntity.updateFallFlyingMovement run on the 26.3 JVM from
    // (0.4, -0.3, 1.1): a climb (Mth.sin in the pull-up) and a dive.
    let cases = [
        (
            -25.3,
            37.7,
            [
                0x3fd2_0750_bb9c_308e,
                0xbfce_177a_a476_8fa8,
                0x3ff1_3a5d_8654_8dc8,
            ],
        ),
        (
            12.9,
            -118.62,
            [
                0x3fde_db16_996e_3d53,
                0xbfd2_5548_1a43_d31f,
                0x3fed_2aff_c5b0_6151,
            ],
        ),
    ];
    for (pitch, yaw, want) in cases {
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 40.0, 20.5)).unwrap();
        player.glider = true;
        player.fall_flying = true;
        player.pitch = pitch;
        player.yaw = yaw;
        player.velocity = Vec3::new(0.4, -0.3, 1.1);
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        w.travel_fall_flying(&mut MoveGate::new()).unwrap();
        let v = w.player.velocity;
        assert_eq!(
            [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()],
            want,
            "{pitch} {yaw}: {v:?}"
        );
    }
}

#[test]
#[ignore = "daily"]
fn a_glide_that_is_not_diving_keeps_the_fall_distance_at_one() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    // Entity.checkFallDistanceAccumulation: capped above -0.5 a tick, not below.
    for (dy, capped) in [(-0.3, true), (-0.49, true), (-0.8, false)] {
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 20.0, 20.5)).unwrap();
        player.glider = true;
        player.fall_flying = true;
        player.velocity = Vec3::new(0.0, dy, 0.6);
        player.fall_distance = 30.0;
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        w.player_movement_tick([0.0, 0.0], false).unwrap();
        assert!(w.player.fall_flying);
        if capped {
            assert!(
                w.player.fall_distance < 2.0,
                "{dy}: {}",
                w.player.fall_distance
            );
        } else {
            assert!(
                w.player.fall_distance > 30.0,
                "{dy}: {}",
                w.player.fall_distance
            );
        }
    }
}
/// Footsteps on the prepared vanilla resources, when they are present:
/// none while standing (gravity still moves the player every tick), one
/// per `nextStep` metre while walking.
#[test]
#[ignore = "daily"]
fn resident_water_landing_cancels_a_preset_fall() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    for wet in [false, true] {
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        if wet {
            let water = assets.registry.default_state("minecraft:water").unwrap();
            for x in 19..=21 {
                for z in 19..=21 {
                    scene.set(x, 3, z, water).unwrap();
                }
            }
        }
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 20.0, 20.5)).unwrap();
        player.fall_distance = 30.0;
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        w.move_self(&mut MoveGate::new(), Vec3::new(0.0, -30.0, 0.0))
            .unwrap();
        assert!(w.player.ground.on_ground);
        assert_eq!(w.player.fluids.was_touching_water, wet);
        if wet {
            assert_eq!(w.player.health.health(), 20.0);
            assert_eq!(w.player.fall_distance, 0.0);
        } else {
            assert!(w.player.health.health() < 20.0);
        }
    }
}
/// `Player.doWaterSplashEffect`: a spectator entering water makes no splash
/// sound, particles or game event; everyone else does.
#[test]
#[ignore = "daily"]
fn a_spectator_entering_water_makes_no_splash() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    for spectator in [false, true] {
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 20.0, 20.5)).unwrap();
        player.spectator = spectator;
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        horizon_world::entity_fluid_tick::EntityFluidTickWorld::water_splash(&mut w).unwrap();
        assert_eq!(w.player.events.is_empty(), spectator);
    }
}
/// A client's own fall (multiplayer, a LAN guest, single player on its
/// server thread) plays its landing but never hurts it: only the server
/// does (`Entity.hurt` on a client), so a server that cancels fall damage
/// leaves the screen still.
#[test]
#[ignore = "daily"]
fn a_clients_own_fall_does_not_hurt_it() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    for client_side in [false, true] {
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 20.0, 20.5)).unwrap();
        player.client_side = client_side;
        player.fall_distance = 30.0;
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        w.move_self(&mut MoveGate::new(), Vec3::new(0.0, -30.0, 0.0))
            .unwrap();
        assert!(w.player.ground.on_ground);
        assert_eq!(w.player.fall_distance, 0.0);
        // LivingEntity.causeFallDamage's sounds play on both sides.
        assert!(w.player.events.iter().any(|e| matches!(
            e,
            MotionEvent::Fall(crate::landing::FallEvent::Sound {
                sound: crate::landing::SceneSound::BigFall,
                ..
            })
        )));
        let hurt = w.player.health.health() < 20.0;
        assert_eq!(hurt, !client_side);
        assert_eq!(w.player.health.hurt_animation_time > 0, !client_side);
        // The server's health then starts the client's hurt animation.
        if client_side {
            w.player.health.flash_health_change(20.0);
            assert_eq!(w.player.health.hurt_animation_time, 0);
            assert_eq!(w.player.health.hurt.cooldown, 0);
            w.player.health.flash_health_change(12.0);
            assert_eq!(w.player.health.hurt_animation_time, 10);
            // LocalPlayer.hurtTo's damageCooldownTime: the HUD's hearts blink.
            assert_eq!(w.player.health.hurt.cooldown, 20);
            w.player.health.flash_health_change(25.0);
            assert_eq!(w.player.health.hurt.cooldown, 10);
        }
    }
}
#[test]
#[ignore = "daily"]
fn slow_falling_clears_the_fall_distance_so_landing_does_not_hurt() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    for slow_falling in [false, true] {
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 9.0, 20.5)).unwrap();
        player.fall_distance = 30.0;
        player.status.slow_falling = slow_falling;
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        for _ in 0..200 {
            if w.player.ground.on_ground {
                break;
            }
            w.player_movement_tick([0.0, 0.0], false).unwrap();
        }
        assert!(w.player.ground.on_ground);
        if slow_falling {
            assert_eq!(w.player.health.health(), 20.0);
            assert_eq!(w.player.fall_distance, 0.0);
        } else {
            assert!(w.player.health.health() < 20.0);
        }
    }
}
#[test]
#[ignore = "daily"]
fn a_server_copy_of_a_riptide_spin_counts_down_and_stands_again() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 2).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
    player.auto_spin_attack_ticks = 2;
    player.auto_spin_attack_dmg = 8.0;
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    w.update_player_pose().unwrap();
    assert_eq!(w.player.pose, crate::pose::Pose::SpinAttack);
    w.player.count_down_server_spin();
    w.update_player_pose().unwrap();
    assert_eq!(w.player.pose, crate::pose::Pose::SpinAttack);
    assert_eq!(w.player.auto_spin_attack_dmg, 8.0);
    w.player.count_down_server_spin();
    assert_eq!(w.player.auto_spin_attack_ticks, 0);
    assert_eq!(w.player.auto_spin_attack_dmg, 0.0);
    w.update_player_pose().unwrap();
    assert_eq!(w.player.pose, crate::pose::Pose::Standing);
    assert_eq!(w.player.dimensions.eye_height, 1.62);
    // Nothing to count once the spin is over.
    w.player.count_down_server_spin();
    assert_eq!(w.player.auto_spin_attack_ticks, 0);
}
/// #3684: `/attribute @s minecraft:scale base set 0.1` on the server's
/// copy of a player. `LivingEntity.tick`'s `refreshDirtyAttributes` runs
/// `onAttributeUpdated(SCALE)`, which calls `refreshDimensions`: the next
/// tick the server checks the player's moves with a 0.06 × 0.18 box, not
/// the 0.6 × 1.8 one that sent a small player back from every wall.
#[test]
#[ignore = "daily"]
fn a_server_copy_takes_its_new_scale_box_the_next_tick() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 2).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    w.server_player_pose_tick().unwrap();
    assert_eq!(w.player.dimensions.width, 0.6);
    assert_eq!(w.player.dimensions.height, 1.8);
    // AttributeCommand's base set, as the server runs it.
    let key = w.player.attributes.key("minecraft:scale").unwrap();
    let handle = w
        .player
        .attributes
        .get_instance(Some(key))
        .unwrap()
        .unwrap();
    w.player
        .attributes
        .edit(&handle, |value, observer| {
            value.set_base_value(0.1, observer)
        })
        .unwrap();
    w.server_player_pose_tick().unwrap();
    assert_eq!(w.player.pose, crate::pose::Pose::Standing);
    assert_eq!(w.player.dimensions.width, 0.6 * 0.1);
    assert_eq!(w.player.dimensions.height, 1.8 * 0.1);
    assert_eq!(w.player.dimensions.eye_height, 1.62 * 0.1);
}
/// #3308: a swimmer who boards a boat stops swimming
/// (`Entity.updateSwimming` needs `!isPassenger()`) and `updatePlayerPose`
/// sits them up, eye at 1.62 instead of the swim's 0.4.
#[test]
#[ignore = "daily"]
fn a_passenger_stops_swimming_and_stands() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 2).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
    player.sprinting = true;
    player.swimming = true;
    player.set_pose(crate::pose::Pose::Swimming).unwrap();
    assert_eq!(player.dimensions.eye_height, 0.4);
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    w.passenger_pose_tick().unwrap();
    assert!(!w.player.swimming);
    assert_eq!(w.player.pose, crate::pose::Pose::Standing);
    assert_eq!(w.player.dimensions.eye_height, 1.62);
}
/// #4509: a cushion seat clips the feet into the floor, so `can_fit_pose`
/// rejects standing and would leave a swim pose; sit up anyway.
#[test]
#[ignore = "daily"]
fn a_swimming_passenger_sits_up_even_when_standing_does_not_fit() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 2).unwrap();
    // Feet inside the floor block, as on a cushion seat (y - 0.35): away
    // from the pyramid the scene's floor tops out at y = 4.
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 3.65, 20.5)).unwrap();
    player.swimming = true;
    player.set_pose(crate::pose::Pose::Swimming).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    assert!(!w.can_fit_pose(crate::pose::Pose::Standing).unwrap());
    w.passenger_pose_tick().unwrap();
    assert!(!w.player.swimming);
    assert_eq!(w.player.pose, crate::pose::Pose::Standing);
}
/// The movement prediction plays a tick again on a copy: what the copy
/// does (sprinting edits the speed attribute) stays in the copy.
#[test]
#[ignore = "daily"]
fn a_copied_player_moves_on_its_own() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let resources = &v.motion;
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
    let walking = player.attribute("minecraft:movement_speed").unwrap();
    let mut copy = player.clone();
    copy.set_sprinting(true).unwrap();
    copy.velocity = Vec3::new(0.2, 0.0, 0.0);
    let sprinting = copy.attribute("minecraft:movement_speed").unwrap();
    assert!(
        (sprinting - walking * 1.3).abs() < 1e-6,
        "{walking} {sprinting}"
    );
    assert_eq!(
        player.attribute("minecraft:movement_speed").unwrap(),
        walking
    );
    assert!(!player.sprinting);
    assert_eq!(player.velocity, Vec3::ZERO);
    // The copy's attributes keep working after the original changes too.
    player.set_sprinting(true).unwrap();
    copy.set_sprinting(false).unwrap();
    assert_eq!(copy.attribute("minecraft:movement_speed").unwrap(), walking);
}
/// #4427: `!isPassenger()` where the swim flag is made, not only where the
/// pose is chosen. The server's copy of a guest who boards a boat while
/// swimming kept `SWIMMING` through `Entity.baseTick`, so it sat in the seat
/// in the 0.6 box with its eye 0.4 above it, at the waterline, where the
/// boat drowned and suffocated a player whose own screen showed them
/// sitting up. Getting out starts the swim again, so it cannot stick off.
#[test]
#[ignore = "daily"]
fn a_riding_guest_keeps_no_swim_in_the_fluid_phase() {
    // The boxes the flag picks, before the prepared resources below: the
    // swim's eye is the one at the waterline. Nothing here needs assets.
    assert_eq!(crate::pose::Pose::Swimming.dimensions(1.0).eye_height, 0.4);
    assert_eq!(crate::pose::Pose::Standing.dimensions(1.0).eye_height, 1.62);
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    let water = assets.registry.default_state("minecraft:water").unwrap();
    for y in 4..=6 {
        scene.set(20, y, 20, water).unwrap();
    }
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    // `fluid_base_tick` samples `was_eye_in_water` before it updates the
    // fluids (`motion/fluids.rs:222`), so a fresh body needs one tick
    // before `is_under_water` can answer: the first call records the eye,
    // the second acts on it. Without that warm-up this test panicked as
    // soon as `.local/` existed, so it never ran for the author or CI.
    w.player.sprinting = true;
    w.fluid_base_tick().unwrap();
    w.fluid_base_tick().unwrap();
    assert!(w.player.is_under_water());
    assert!(w.player.swimming);
    // In the boat: `Entity.updateSwimming` keeps no swim for a passenger.
    // This is the assertion that fails on main, where the term is missing.
    w.player.is_passenger = true;
    w.fluid_base_tick().unwrap();
    assert!(!w.player.swimming);
    // Out of the boat the swim starts again.
    w.player.is_passenger = false;
    w.fluid_base_tick().unwrap();
    assert!(w.player.swimming);
}
#[test]
#[ignore = "daily"]
fn a_riptide_spin_lies_flat_and_ends_on_the_first_entity_it_touches() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 2).unwrap();
    let ahead = Aabb::new(Vec3::new(20.2, 9.0, 21.5), Vec3::new(20.8, 11.0, 22.1));
    for target in [None, Some(ahead)] {
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
        player.auto_spin_attack_ticks = 20;
        player.auto_spin_attack_dmg = 8.0;
        player.velocity = Vec3::new(0.0, 0.0, 1.0);
        player.spin_targets = target.map(|b| (7, b)).into_iter().collect();
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        w.simulation_tick([0.0, 0.0], false).unwrap();
        if target.is_some() {
            // Touched: the hit, no more spin, bounced back at a fifth.
            assert_eq!(w.player.spin_hit, Some(7));
            assert_eq!(w.player.auto_spin_attack_ticks, 0);
            assert!(w.player.velocity.z < 0.0, "{:?}", w.player.velocity);
            assert_eq!(w.player.pose, crate::pose::Pose::Standing);
        } else {
            assert_eq!(w.player.spin_hit, None);
            assert_eq!(w.player.auto_spin_attack_ticks, 19);
            assert_eq!(w.player.pose, crate::pose::Pose::SpinAttack);
            assert_eq!(w.player.dimensions.height, 0.6);
        }
    }
}
#[test]
#[ignore = "daily"]
fn a_landing_glide_lasts_a_tick_so_a_jump_keeps_its_speed() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 2).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.2, 20.5)).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    w.move_self(&mut MoveGate::new(), Vec3::new(0.0, -1.0, 0.0))
        .unwrap();
    assert!(w.player.ground.on_ground);
    w.player.set_sprinting(true).unwrap();
    w.player.glider = true;
    w.player.fall_flying = true;
    w.player.velocity = Vec3::new(0.0, 0.0, 1.5);
    // The landed glide's extra tick: the jump's sprint boost, no ground friction.
    w.player_movement_tick([0.0, 1.0], true).unwrap();
    assert!(w.player.fall_flying);
    assert!(w.player.velocity.y > 0.3, "jumped: {:?}", w.player.velocity);
    assert!(
        w.player.velocity.z > 1.5,
        "kept its speed: {:?}",
        w.player.velocity
    );
    // Then the predicted clear (an integrated server has no SetEntityData
    // round trip: the client plays it a tick later).
    w.player_movement_tick([0.0, 1.0], true).unwrap();
    assert!(!w.player.fall_flying);
}

/// #7616: on someone else's server the glide flag is `DATA_SHARED_FLAGS_ID`'s.
/// `LivingEntity.updateFallFlying` clears it on the server only and
/// `LocalPlayer` never ends a glide itself, so a client that predicts the
/// clear clears it before the server's `SetEntityData` and the movement of
/// the tick after the landing is air travel where vanilla still glides.
#[test]
#[ignore = "daily"]
fn a_server_client_glides_until_the_servers_flag_clears() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 2).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.2, 20.5)).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    w.move_self(&mut MoveGate::new(), Vec3::new(0.0, -1.0, 0.0))
        .unwrap();
    assert!(w.player.ground.on_ground);
    w.player.set_sprinting(true).unwrap();
    w.player.glider = true;
    w.player.fall_flying = true;
    w.player.server_owns_glide = true;
    w.player.velocity = Vec3::new(0.0, 0.0, 1.5);
    // The landing tick's jump, gliding: vy = (0.42 - 0.08 * 0.25) * 0.98.
    w.player_movement_tick([0.0, 1.0], true).unwrap();
    assert!(w.player.fall_flying);
    assert!(
        (w.player.velocity.y - 0.392).abs() < 1e-6,
        "jump: {:?}",
        w.player.velocity
    );
    // The tick after it: the local `canGlide` failed on the landing tick,
    // but no `SetEntityData` has arrived, so the glide goes on — vanilla's
    // `LocalPlayer` glides here too. Its travel is the glide's
    // (0.392 - 0.02) * 0.98, not the air's (0.392 - 0.08) * 0.98.
    w.player_movement_tick([0.0, 1.0], true).unwrap();
    assert!(w.player.fall_flying, "only the server's flag ends a glide");
    assert!(
        (w.player.velocity.y - 0.36456).abs() < 1e-5,
        "glide travel: {:?}",
        w.player.velocity
    );
    // The server's cleared flag (0x80 gone), handled between ticks.
    w.player.fall_flying = false;
    w.player_movement_tick([0.0, 1.0], true).unwrap();
    assert!(!w.player.fall_flying);
    assert!(
        (w.player.velocity.y - 0.2788688).abs() < 1e-5,
        "air travel: {:?}",
        w.player.velocity
    );
}

#[test]
#[ignore = "daily"]
fn resident_water_and_lava_are_not_treated_as_air() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        eprintln!("skipping resident water/lava test: prepared resources unavailable");
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    let water = assets.registry.default_state("minecraft:water").unwrap();
    for x in 19..=21 {
        for y in 3..=7 {
            for z in 19..=21 {
                scene.set(x, y, z, water).unwrap();
            }
        }
    }
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    w.controlled_simulation_tick(Default::default()).unwrap();
    assert!(
        MovementStatisticsWorld::in_water(&mut w).unwrap(),
        "resident water must reach the player movement adapter"
    );
    assert!(MovementStatisticsWorld::eyes_in_water(&mut w).unwrap());
    assert!(
        !w.player.fluids.was_eye_in_water,
        "the first controlled tick must run only one fluid base phase"
    );
    assert!(
        (w.player.velocity.y + 0.005).abs() < 1e-5,
        "water drag/gravity must replace air travel: {:?}",
        w.player.velocity
    );
    let lava = assets.registry.default_state("minecraft:lava").unwrap();
    for x in 19..=21 {
        for y in 3..=7 {
            for z in 19..=21 {
                w.scene.set(x, y, z, lava).unwrap();
            }
        }
    }
    w.controlled_simulation_tick(Default::default()).unwrap();
    assert!(
        w.player.health.health() < 20.0,
        "lava must damage a survival player"
    );
    // Water extinguishes, swimming is a real pose, and deep water consumes air.
    for x in 19..=21 {
        for y in 3..=7 {
            for z in 19..=21 {
                w.scene.set(x, y, z, water).unwrap();
            }
        }
    }
    w.player.fire_ticks = 160;
    w.controlled_simulation_tick(Default::default()).unwrap();
    assert!(w.player.fire_ticks <= 0);
    w.player.air = -19;
    w.player.health.hurt.cooldown = 0;
    let hp = w.player.health.health();
    w.controlled_simulation_tick(Default::default()).unwrap();
    assert_eq!(w.player.air, 0);
    assert!(w.player.health.health() < hp, "drowning must hurt");
    w.player.invulnerable = true;
    let hp = w.player.health.health();
    w.player.air = -19;
    w.controlled_simulation_tick(Default::default()).unwrap();
    assert_eq!(w.player.health.health(), hp);
    w.player.invulnerable = false;
    w.player.air = 300;
    w.controlled_simulation_tick(crate::sprint::Keys {
        forward: true,
        sprint: true,
        ..Default::default()
    })
    .unwrap();
    assert!(w.player.sprinting);
    assert!(
        !w.player.swimming,
        "sprint input follows the swimming phase"
    );
    w.controlled_simulation_tick(crate::sprint::Keys {
        forward: true,
        sprint: true,
        ..Default::default()
    })
    .unwrap();
    assert!(w.player.swimming);
    assert_eq!(w.player.pose, crate::pose::Pose::Swimming);
    // Direct fire contact ignites; resistance prevents damage but does not hide flames.
    let fire = assets.registry.default_state("minecraft:fire").unwrap();
    for x in 19..=21 {
        for y in 3..=7 {
            for z in 19..=21 {
                w.scene.set(x, y, z, fire).unwrap();
            }
        }
    }
    w.player.position = Vec3::new(20.5, 4.0, 20.5);
    w.player.velocity = Vec3::ZERO;
    w.player.fire_ticks = -20;
    w.player.health.hurt.cooldown = 0;
    w.player.ticks_frozen = 140;
    let hp = w.player.health.health();
    w.controlled_simulation_tick(Default::default()).unwrap();
    assert!(
        w.player.fire_ticks < 0,
        "direct fire hurts during the immunity delay"
    );
    assert_eq!(
        w.player.ticks_frozen, 0,
        "fire contact clears freeze during the delay"
    );
    assert!(w.player.health.health() < hp);
    w.player.status.fire_resistance = true;
    w.player.health.hurt.cooldown = 0;
    let hp = w.player.health.health();
    w.controlled_simulation_tick(Default::default()).unwrap();
    assert_eq!(w.player.health.health(), hp);
    w.player.invulnerable = true;
    w.player.fire_ticks = -20;
    w.player.ticks_frozen = 140;
    w.controlled_simulation_tick(Default::default()).unwrap();
    assert_eq!(w.player.ticks_frozen, 0);
    assert_eq!(
        w.player.fire_ticks, -19,
        "creative preserves the fire-contact delay"
    );
    for _ in 0..18 {
        w.apply_fire_contact().unwrap();
    }
    assert_eq!(w.player.fire_ticks, -1);
    w.apply_fire_contact().unwrap();
    assert_eq!(
        w.player.fire_ticks, 1,
        "creative clamps only after ignition"
    );
    w.player.ticks_frozen = 140;
    w.player.ignite_for_ticks(160).unwrap();
    assert_eq!(w.player.ticks_frozen, 0);
    assert_eq!(w.player.fire_ticks, 1);
    w.player.invulnerable = false;
    // Sprint swimming must keep full input, not the dry crawl/sneak multiplier.
    for x in 19..=21 {
        for y in 2..=7 {
            for z in 19..=30 {
                w.scene.set(x, y, z, water).unwrap();
            }
        }
    }
    w.player.position = Vec3::new(20.5, 4.0, 20.5);
    w.player.velocity = Vec3::ZERO;
    w.player.local_crouching = false;
    w.player.swimming = true;
    w.player.sprinting = true;
    w.player.set_pose(crate::pose::Pose::Swimming).unwrap();
    for _ in 0..30 {
        w.controlled_simulation_tick(crate::sprint::Keys {
            forward: true,
            sprint: true,
            ..Default::default()
        })
        .unwrap();
    }
    assert!(
        w.player.velocity.z > 0.165,
        "native sprint swim speed must not receive crawl slowdown: {:?}",
        w.player.velocity
    );
    assert!(!w.player.moving_slowly());
    w.player.position = Vec3::new(20.5, 4.0, 20.5);
    w.player.velocity = Vec3::ZERO;
    w.player.pitch = 45.0;
    w.controlled_simulation_tick(crate::sprint::Keys {
        forward: true,
        sprint: true,
        ..Default::default()
    })
    .unwrap();
    assert!(
        w.player.velocity.y < -0.04,
        "look down must steer a swimming player downward"
    );
    w.player.position = Vec3::new(20.5, 4.0, 20.5);
    w.player.velocity = Vec3::ZERO;
    w.player.pitch = -45.0;
    w.controlled_simulation_tick(crate::sprint::Keys {
        forward: true,
        sprint: true,
        ..Default::default()
    })
    .unwrap();
    assert!(
        w.player.velocity.y > 0.02,
        "look up must steer a submerged swimmer upward"
    );
    let extra = horizon_core::nbt::Tag::compound([
        ("Fire", horizon_core::nbt::Tag::Short(160)),
        ("Air", horizon_core::nbt::Tag::Short(8)),
    ]);
    let mut level = crate::save::Level {
        health: 13.0,
        food: 11,
        saturation: 2.25,
        player_extra: Some(extra),
        flying: true,
        ..Default::default()
    };
    let mut pending = true;
    w.player
        .load_saved_state_once(&mut pending, &level)
        .unwrap();
    assert_eq!((w.player.fire_ticks, w.player.air), (160, 8));
    assert!(w.player.flying);
    assert_eq!(
        (
            w.player.health.health(),
            w.player.health.food.food,
            w.player.health.food.saturation
        ),
        (13.0, 11, 2.25)
    );
    let mut respawn = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    respawn.load_saved_state_once(&mut pending, &level).unwrap();
    assert_eq!((respawn.fire_ticks, respawn.air), (-20, 300));
    assert!(!respawn.flying);
    assert_eq!(
        (respawn.health.health(), respawn.health.food.food),
        (20.0, 20)
    );
    // Reopening another world restores its state, including a saved death.
    pending = true;
    level.health = 0.0;
    w.player
        .load_saved_state_once(&mut pending, &level)
        .unwrap();
    assert!(w.player.health.dead_or_dying());
    assert!(!pending);
    w.player.fluids.was_touching_water = false;
    assert!(
        w.player.moving_slowly(),
        "dry crawling must still slow input"
    );
}

#[test]
#[ignore = "daily"]
fn resident_swimming_uses_previous_eyes_and_requires_water_at_the_feet() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    let water = assets.registry.default_state("minecraft:water").unwrap();
    for y in 4..=6 {
        scene.set(20, y, 20, water).unwrap();
    }
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    w.player.local_crouching = true;
    let sprint = crate::sprint::Keys {
        forward: true,
        sprint: true,
        ..Default::default()
    };
    w.fluid_base_tick().unwrap();
    assert!(w.player.is_eye_in_fluid(WATER));
    assert!(!w.player.is_under_water());
    w.player.sprint_input_tick(sprint).unwrap();
    assert!(
        !w.player.sprinting,
        "new eye immersion cannot bypass crouching"
    );
    w.fluid_base_tick().unwrap();
    assert!(w.player.is_under_water());
    w.player.sprint_input_tick(sprint).unwrap();
    assert!(w.player.sprinting);
    assert!(!w.player.swimming, "sprint starts after the base tick");
    w.fluid_base_tick().unwrap();
    assert!(w.player.swimming);
    let air = assets.registry.default_state("minecraft:air").unwrap();
    w.scene.set(20, 4, 20, air).unwrap();
    w.player.swimming = false;
    w.fluid_base_tick().unwrap();
    assert!(w.player.is_under_water());
    assert!(
        !w.player.swimming,
        "starting a swim requires water in the feet cell"
    );
    w.player.swimming = true;
    w.fluid_base_tick().unwrap();
    assert!(
        w.player.swimming,
        "an existing swim only requires touching water"
    );
    w.player.flying = true;
    w.fluid_base_tick().unwrap();
    assert!(!w.player.swimming);
    w.player.flying = false;
    w.player.spectator = true;
    w.scene.set(20, 4, 20, water).unwrap();
    w.fluid_base_tick().unwrap();
    assert!(
        w.player.swimming,
        "spectator flight is applied later in aiStep"
    );
}

#[test]
#[ignore = "daily"]
fn resident_fire_precedes_first_tick_lava_and_water_extinguishing() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    for name in ["minecraft:lava", "minecraft:water"] {
        let mut scene = World::test_scene(assets, render, 3).unwrap();
        let fluid = assets.registry.default_state(name).unwrap();
        scene.set(20, 4, 20, fluid).unwrap();
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        player.fire_ticks = 160;
        player.fall_distance = 8.0;
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        w.fluid_base_tick().unwrap();
        assert_eq!(
            w.player.health.health(),
            19.0,
            "base on-fire damage: {name}"
        );
        if name == "minecraft:lava" {
            assert_eq!(w.player.fire_ticks, 159);
            assert_eq!(w.player.fall_distance, 8.0, "first tick excludes lava");
            w.fluid_base_tick().unwrap();
            assert_eq!(
                w.player.fall_distance, 4.0,
                "later ticks halve exactly once"
            );
        } else {
            assert_eq!(w.player.fire_ticks, 0, "water extinguishes after the hurt");
            assert_eq!(w.player.fall_distance, 0.0);
            w.player.fire_ticks = -7;
            w.fluid_base_tick().unwrap();
            assert_eq!(
                w.player.fire_ticks, -7,
                "clearing fire retains negative delay"
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn resident_bubble_columns_and_nautilus_preserve_their_air_rules() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    let water = assets.registry.default_state("minecraft:water").unwrap();
    scene.set(20, 4, 20, water).unwrap();
    scene.set(20, 5, 20, water).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    for (extra_effect, expected_air) in [
        (None, -19),
        (Some("water_breathing"), -15),
        (Some("conduit_power"), -15),
    ] {
        let mut effects = crate::effects::PlayerEffects::default();
        effects.add(crate::effects::EffectInstance::new(
            "breath_of_the_nautilus",
            200,
            0,
        ));
        if let Some(id) = extra_effect {
            effects.add(crate::effects::EffectInstance::new(id, 200, 0));
        }
        w.player.status = effects.status();
        assert!(w.player.status.breath_of_the_nautilus);
        w.player.air = -19;
        w.fluid_base_tick().unwrap();
        w.tick_environment_damage().unwrap();
        assert_eq!(w.player.air, expected_air);
        assert_eq!(w.player.health.health(), 20.0);
    }
    w.player.status.water_breathing = false;
    for (name, nautilus) in [
        ("minecraft:bubble_column", false),
        ("minecraft:bubble_column", true),
        ("minecraft:air", true),
    ] {
        w.scene
            .set(20, 5, 20, assets.registry.default_state(name).unwrap())
            .unwrap();
        w.player.status.breath_of_the_nautilus = nautilus;
        w.player.air = -19;
        w.fluid_base_tick().unwrap();
        w.tick_environment_damage().unwrap();
        assert_eq!(w.player.air, -15, "{name}, nautilus={nautilus}");
        assert_eq!(w.player.health.health(), 20.0);
    }
    w.scene.set(20, 5, 20, water).unwrap();
    w.player.status.breath_of_the_nautilus = false;
    w.player.air = -19;
    w.fluid_base_tick().unwrap();
    w.tick_environment_damage().unwrap();
    assert_eq!(w.player.air, 0);
    assert_eq!(w.player.health.health(), 18.0);
}

#[test]
#[ignore = "daily"]
fn resident_mixed_fluids_use_lava_height_for_ground_jumps() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    for (water_level, lava_level, ground_jump) in [(0, 6, true), (6, 0, false)] {
        let mut scene = World::test_scene(assets, render, 3).unwrap();
        for (name, level, x) in [
            ("minecraft:water", water_level, 20),
            ("minecraft:lava", lava_level, 21),
        ] {
            let mut state = assets
                .registry
                .state(assets.registry.default_state(name).unwrap())
                .unwrap()
                .clone();
            state.properties.insert("level".into(), level.to_string());
            scene
                .set(x, 4, 20, assets.registry.id(&state).unwrap())
                .unwrap();
        }
        let mut player = PlayerMotion::new(resources, Vec3::new(21.0, 4.0, 20.5)).unwrap();
        player.ground.on_ground = true;
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        w.fluid_base_tick().unwrap();
        assert!(w.player.fluids.was_touching_water && w.player.is_in_lava());
        assert_eq!(w.player.fluid_height(LAVA) <= 0.4, ground_jump);
        assert_eq!(w.player.fluid_height(WATER) > 0.4, ground_jump);
        w.dry_tick(TickInput::PreparedLocal {
            direction: [0.0; 2],
            jumping: true,
        })
        .unwrap();
        assert_eq!(w.player.no_jump_delay == 10, ground_jump);
    }
}

/// The player's fluid heights, which `entity_fluid_heights` reads: lava sources at x 20 and
/// water sources at x 21, y 4..=5 (each surface at 5 + 8/9). Standing at y 4 the player is
/// that surface less 4 deep in each fluid it touches, from its own bottom, as a mob is
/// (`mobs::living::fluid_heights`): lava, water, both where they meet (a tie), or neither.
#[test]
#[ignore = "daily"]
fn resident_fluid_heights_are_measured_from_the_feet() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        eprintln!("Prepared resources unavailable; the player fluid heights did not run");
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    for (name, x) in [("minecraft:lava", 20), ("minecraft:water", 21)] {
        let source = assets.registry.default_state(name).unwrap();
        for y in 4..=5 {
            scene.set(x, y, 20, source).unwrap();
        }
    }
    // A source's height is the `f32` 8/9 (`8 / 9F`), so the depth is 1 + that, in `f64`.
    let depth = 1.0 + f64::from(8.0_f32 / 9.0);
    // (x, water height, lava height)
    for (x, water, lava) in [
        (20.5, 0.0, depth),
        (21.5, depth, 0.0),
        (21.0, depth, depth),
        (25.5, 0.0, 0.0),
    ] {
        let mut player = PlayerMotion::new(resources, Vec3::new(x, 4.0, 20.5)).unwrap();
        player.ground.on_ground = true;
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        w.fluid_base_tick().unwrap();
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let got_water = w.player.fluid_height(WATER);
        let got_lava = w.player.fluid_height(LAVA);
        assert!(near(got_water, water), "water at x {x}: {got_water}");
        assert!(near(got_lava, lava), "lava at x {x}: {got_lava}");
        assert_eq!(w.player.fluids.was_touching_water, water > 0.0, "x {x}");
        assert_eq!(w.player.is_in_lava(), lava > 0.0, "x {x}");
    }
}

#[test]
#[ignore = "daily"]
fn footsteps_follow_the_walked_distance_not_the_tick() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render) = (&v.assets, &v.render);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    let (binding, resources) = (&v.collision, &v.motion);
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    let mut steps = |player: &mut PlayerMotion, keys: crate::sprint::Keys, ticks: usize| {
        let mut count = 0;
        for _ in 0..ticks {
            MotionWorld {
                tag_items: None,
                block_registries: None,
                loop_context: crate::test_resources::server_loop_context(),
                damage_hooks: &crate::health::DamageHooks::NONE,
                entities: &[],
                player: &mut *player,
                scene: &mut scene,
                assets,
                collision_binding: binding,
                resources,
                health: &resources.health,
            }
            .controlled_simulation_tick(keys)
            .unwrap();
            count += std::mem::take(&mut player.events)
                .iter()
                .filter(|e| matches!(e, MotionEvent::Emission { .. }))
                .count();
        }
        count
    };
    // Settle onto the floor, then stand still.
    steps(&mut player, Default::default(), 20);
    assert!(player.ground.on_ground);
    assert_eq!(steps(&mut player, Default::default(), 100), 0);
    let start = player.position;
    let forward = crate::sprint::Keys {
        forward: true,
        ..Default::default()
    };
    let walked = steps(&mut player, forward, 100);
    let metres = player.position.subtract(start).length() * 0.6;
    assert!(metres > 5.0, "walked {metres}");
    assert!(
        (walked as f64 - metres).abs() <= 1.5,
        "{walked} steps over {metres} step-metres"
    );
}
/// Vanilla sprint is a state, not a held key: `LocalPlayer.aiStep` starts
/// it with the sprint key and stops it only on a vanilla stop condition.
/// Releasing the key while swimming keeps the sprint and its swim speed.
#[test]
#[ignore = "daily"]
fn a_sprint_survives_the_sprint_key_coming_up_while_swimming() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let water = assets.registry.default_state("minecraft:water").unwrap();
    let forward = crate::sprint::Keys {
        forward: true,
        ..Default::default()
    };
    let sprinting = crate::sprint::Keys {
        sprint: true,
        ..forward
    };
    // The same swim with the sprint key held, and with the key released
    // once the sprint has started: the speed must not change.
    let mut swims = Vec::new();
    for release in [false, true] {
        let mut scene = World::test_scene(assets, render, 3).unwrap();
        for x in 19..=21 {
            for y in 4..=8 {
                for z in 19..=31 {
                    scene.set(x, y, z, water).unwrap();
                }
            }
        }
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        let mut speed = Vec::new();
        for tick in 0..12 {
            // The key comes up on the fourth tick, after the sprint has
            // started and the player swims.
            let keys = if release && tick >= 4 {
                forward
            } else {
                sprinting
            };
            w.controlled_simulation_tick(keys).unwrap();
            speed.push(w.player.velocity.z);
        }
        assert!(
            w.player.sprinting,
            "the sprint state survives the sprint key in water"
        );
        assert!(w.player.swimming, "a sprinting player in water swims");
        swims.push(speed);
    }
    assert_eq!(
        swims[0], swims[1],
        "the swim speed must not drop when the sprint key comes up"
    );
}

/// Vanilla: `LivingEntity.jumpFromGround` adds its 0.2 forward impulse
/// while the sprint state holds, so a jump taken after the sprint key
/// comes up is still a sprint jump.
#[test]
#[ignore = "daily"]
fn a_sprint_jump_keeps_its_impulse_when_the_sprint_key_comes_up() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    let forward = crate::sprint::Keys {
        forward: true,
        ..Default::default()
    };
    let sprinting = crate::sprint::Keys {
        sprint: true,
        ..forward
    };
    for _ in 0..20 {
        w.controlled_simulation_tick(sprinting).unwrap();
    }
    assert!(w.player.ground.on_ground, "the player settles on the floor");
    assert!(w.player.sprinting, "the sprint key starts the sprint");
    let running = w.player.velocity.z;
    // The sprint key comes up and the player jumps.
    w.controlled_simulation_tick(crate::sprint::Keys {
        jump: true,
        ..forward
    })
    .unwrap();
    assert!(
        w.player.sprinting,
        "the sprint state survives the sprint key in the air"
    );
    assert!(
        w.player.velocity.z > running + 0.05,
        "the sprint jump keeps its forward impulse: {:?} vs {running}",
        w.player.velocity
    );
}

/// Vanilla `LocalPlayer.aiStep`: an item in use whose `use_effects` do not
/// allow sprinting (a raised shield) keeps the sprint key, and a
/// double-tap of forward, from starting a sprint; an item that allows it
/// (a spear) does not.
#[test]
#[ignore = "daily"]
fn an_item_in_use_stops_a_sprint_from_starting_unless_it_allows_sprinting() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    let forward = crate::sprint::Keys {
        forward: true,
        ..Default::default()
    };
    let sprint_key = crate::sprint::Keys {
        sprint: true,
        ..forward
    };
    w.player.item_use_speed = Some(0.2);
    w.player.item_use_slows_sprint = true;
    for _ in 0..10 {
        w.controlled_simulation_tick(sprint_key).unwrap();
    }
    assert!(!w.player.sprinting, "a raised shield stops the sprint key");
    // Forward, off, forward again: the double-tap would sprint.
    for keys in [forward, Default::default(), forward, forward] {
        w.controlled_simulation_tick(keys).unwrap();
    }
    assert!(!w.player.sprinting, "a raised shield stops a double-tap");
    // A spear's use_effects allow sprinting.
    w.player.item_use_speed = Some(1.0);
    w.player.item_use_slows_sprint = false;
    w.controlled_simulation_tick(sprint_key).unwrap();
    assert!(w.player.sprinting, "an item that can_sprint lets it start");
}

/// Vanilla `LocalPlayer.isSprintingPossible`: blindness
/// (`Player.isMobilityRestricted`) keeps the sprint key from starting a
/// sprint and stops a running one (`shouldStopRunSprinting`).
#[test]
#[ignore = "daily"]
fn blindness_stops_a_sprint_and_keeps_one_from_starting() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    let sprint_key = crate::sprint::Keys {
        forward: true,
        sprint: true,
        ..Default::default()
    };
    w.controlled_simulation_tick(sprint_key).unwrap();
    assert!(w.player.sprinting);
    w.player.mobility_restricted = true;
    w.controlled_simulation_tick(sprint_key).unwrap();
    assert!(!w.player.sprinting, "blindness stops a running sprint");
    for _ in 0..10 {
        w.controlled_simulation_tick(sprint_key).unwrap();
    }
    assert!(
        !w.player.sprinting,
        "blindness keeps the sprint key from starting one"
    );
    w.player.mobility_restricted = false;
    w.controlled_simulation_tick(sprint_key).unwrap();
    assert!(w.player.sprinting);
}

#[test]
#[ignore = "daily"]
fn server_forced_flight_survives_without_permission_to_toggle_it() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        eprintln!("skipped forced flight: prepared resources unavailable");
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    let start = Vec3::new(20.5, 20.0, 20.5);
    let mut player = PlayerMotion::new(resources, start).unwrap();
    player.may_fly = false;
    player.flying = true;
    player.flying_speed = 0.0;
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    // Idle, movement in all directions, and repeated jump presses must
    // neither cancel server-forced flight nor move a zero-speed flyer.
    for tick in 0..60 {
        w.controlled_simulation_tick(crate::sprint::Keys {
            forward: tick % 4 == 0,
            backward: tick % 4 == 1,
            left: tick % 4 == 2,
            right: tick % 4 == 3,
            jump: tick % 2 == 0,
            shift: tick % 3 == 0,
            ..Default::default()
        })
        .unwrap();
        assert!(w.player.flying, "server-forced flight ended on tick {tick}");
        assert_eq!(w.player.position, start);
        assert_eq!(w.player.velocity, Vec3::ZERO);
    }
    // A later abilities update from the server can still end flight.
    w.player.flying = false;
    for _ in 0..3 {
        w.controlled_simulation_tick(crate::sprint::Keys::default())
            .unwrap();
    }
    assert!(!w.player.flying);
    assert!(w.player.position.y < start.y);
}

/// Vanilla `Player.getFlyingSpeed`: a flying player moves at the ability
/// speed, doubled while the sprint state holds. The sprint key starts the
/// state; releasing it does not change the speed.
#[test]
#[ignore = "daily"]
fn the_flight_speed_follows_the_sprint_state_not_the_sprint_key() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 3).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    w.player.may_fly = true;
    w.player.flying = true;
    let forward = crate::sprint::Keys {
        forward: true,
        ..Default::default()
    };
    let sprinting = crate::sprint::Keys {
        sprint: true,
        ..forward
    };
    assert_eq!(
        w.flying_speed().unwrap(),
        0.05,
        "a flying player without a sprint moves at the ability speed"
    );
    // The sprint key starts the sprint, and the state doubles the speed.
    w.controlled_simulation_tick(sprinting).unwrap();
    assert!(w.player.sprinting, "the sprint key starts the sprint");
    assert_eq!(
        w.flying_speed().unwrap(),
        0.1,
        "a sprinting player flies at twice the ability speed"
    );
    // Releasing the sprint key keeps the state, so the speed stays.
    w.controlled_simulation_tick(forward).unwrap();
    assert!(
        w.player.sprinting,
        "the sprint state survives the sprint key in the air"
    );
    assert_eq!(
        w.flying_speed().unwrap(),
        0.1,
        "the flight speed follows the sprint state, not the sprint key"
    );
}
/// #9858: `LivingEntity.tick` counts `fallFlyTicks` after `aiStep`, so
/// `updateFallFlying` sees last tick's count plus one: the glider wears
/// on the 20th glide tick, not the 19th.
#[test]
#[ignore = "daily"]
fn a_glider_wears_on_the_twentieth_glide_tick() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 2).unwrap();
    let mut player = PlayerMotion::new(resources, Vec3::new(20.5, 100.0, 20.5)).unwrap();
    player.glider = true;
    player.fall_flying = true;
    let mut w = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut player,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    for tick in 1..=19 {
        w.player_movement_tick([0.0, 0.0], false).unwrap();
        assert!(w.player.fall_flying, "still gliding on tick {tick}");
        assert_eq!(w.player.glider_wear, 0, "no wear by tick {tick}");
    }
    w.player_movement_tick([0.0, 0.0], false).unwrap();
    assert_eq!(
        w.player.glider_wear, 1,
        "the 20th glide tick wears the glider"
    );
    assert_eq!(w.player.fall_fly_ticks, 20);
}
/// #9817: `LivingEntity.travelInAir`: a client (`level.isClientSide()`) whose
/// chunk below is not loaded skips gravity and falls at 0.1 a tick
/// (above `getMinY`), before the 0.98 vertical drag.
#[test]
#[ignore = "daily"]
fn a_client_over_an_unloaded_chunk_falls_at_a_tenth() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    for (client_side, want) in [
        (true, -0.1 * f64::from(0.98_f32)),
        (false, -0.0784000015258789),
    ] {
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        // Far outside the test scene's chunks.
        let at = Vec3::new(10_000.5, 20.0, 10_000.5);
        assert!(!scene.chunks.contains_key(&(10_000 >> 4, 10_000 >> 4)));
        let mut player = PlayerMotion::new(resources, at).unwrap();
        player.client_side = client_side;
        MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        }
        .controlled_simulation_tick(Default::default())
        .unwrap();
        assert_eq!(player.velocity.y, want, "client_side={client_side}");
    }
}
/// `LocalPlayer.isSuppressingSlidingDownLadder` is `!abilities.flying && super...()`, where
/// `LivingEntity`'s is `isShiftKeyDown()`: a flying player on the client keeps sliding down a
/// ladder whether it sneaks or not. The server's player (`ServerPlayer` has no override) only
/// looks at the sneak.
#[test]
#[ignore = "daily"]
fn a_flying_client_does_not_stop_sliding_down_a_ladder_by_sneaking() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    // (client_side, flying, sneaking, suppressing)
    for (client_side, flying, sneaking, want) in [
        (true, false, true, true),
        (true, false, false, false),
        (true, true, true, false),
        (true, true, false, false),
        (false, false, true, true),
        (false, true, true, true),
        (false, true, false, false),
    ] {
        let mut scene = World::test_scene(assets, render, 2).unwrap();
        let mut player = PlayerMotion::new(resources, Vec3::new(0.5, 64.0, 0.5)).unwrap();
        player.client_side = client_side;
        player.flying = flying;
        player.shift_key_down = sneaking;
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        assert_eq!(
            w.suppressing_ladder_slide().unwrap(),
            want,
            "client_side={client_side} flying={flying} sneaking={sneaking}"
        );
    }
}

/// `Player.getBlockSpeedFactor`: `!abilities.flying && !isFallFlying() ?
/// super.getBlockSpeedFactor() : 1.0F` (#16176). A creative player flying
/// at y -59.8 over soul sand (its top at -60.125) or gliding there moves as
/// it does over grass; walking on the soul sand it keeps 0.4 of its speed.
#[test]
#[ignore = "daily"]
fn a_flying_or_gliding_player_is_not_slowed_by_soul_sand() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 2).unwrap();
    // How much of a 0.1 step east the move leaves in the velocity.
    let mut kept = |floor: &str, y: f64, flying: bool, gliding: bool| {
        let state = assets.registry.default_state(floor).unwrap();
        scene.set(20, -61, 20, state).unwrap();
        let mut player = PlayerMotion::new(resources, Vec3::new(20.5, y, 20.5)).unwrap();
        (player.may_fly, player.flying) = (flying, flying);
        (player.glider, player.fall_flying) = (gliding, gliding);
        player.velocity = Vec3::new(0.1, 0.0, 0.0);
        let mut w = MotionWorld {
            tag_items: None,
            block_registries: None,
            loop_context: crate::test_resources::server_loop_context(),
            damage_hooks: &crate::health::DamageHooks::NONE,
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets,
            collision_binding: binding,
            resources,
            health: &resources.health,
        };
        w.move_self(&mut MoveGate::new(), Vec3::new(0.1, 0.0, 0.0))
            .unwrap();
        w.player.velocity.x / 0.1
    };
    let (soul_sand, grass) = ("minecraft:soul_sand", "minecraft:grass_block");
    assert_eq!(kept(grass, -59.8, true, false), 1.0);
    assert_eq!(kept(soul_sand, -59.8, true, false), 1.0, "flying");
    assert_eq!(kept(soul_sand, -59.8, false, true), 1.0, "gliding");
    let walking = kept(soul_sand, -60.125, false, false);
    assert!((walking - 0.4).abs() < 1.0e-6, "walking: {walking}");
}

#[test]
#[ignore = "daily"]
fn stationary_replayed_tick_fast_path_matches_full_local_tick() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let build_scene = || {
        let mut s = World::test_scene(assets, render, 2).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                s.set(x, 63, z, stone).unwrap();
            }
        }
        s
    };
    let mut scene = build_scene();
    macro_rules! mw {
        ($player:expr, $scene:expr) => {
            MotionWorld {
                tag_items: None,
                block_registries: None,
                loop_context: crate::test_resources::server_loop_context(),
                damage_hooks: &crate::health::DamageHooks::NONE,
                entities: &[],
                player: $player,
                scene: $scene,
                assets,
                collision_binding: binding,
                resources,
                health: &resources.health,
            }
        };
    }
    let mut seed = PlayerMotion::new(resources, Vec3::new(20.5, 64.0, 20.5)).unwrap();
    seed.client_side = true;
    for _ in 0..2 {
        mw!(&mut seed, &mut scene)
            .local_tick(crate::sprint::Keys::default(), |world, _, _| {
                world.player.set_sprinting(false)
            })
            .unwrap();
    }

    let mut fast = seed.clone();
    let mut full = seed;
    let mut scene_full = build_scene();
    for tick in 0..5 {
        assert!(
            mw!(&mut fast, &mut scene)
                .try_fast_stationary_replayed_tick(crate::sprint::Keys::default(), false)
                .unwrap(),
            "tick {tick} must hit fast path"
        );
        full.replaying = true;
        let full_before_events = full.events.len();
        mw!(&mut full, &mut scene_full)
            .local_tick(crate::sprint::Keys::default(), |world, _, _| {
                world.player.set_sprinting(false)
            })
            .unwrap();
        full.replaying = false;

        assert_eq!(fast.position, full.position, "position tick {tick}");
        assert_eq!(
            fast.old_position, full.old_position,
            "old_position tick {tick}"
        );
        assert_eq!(fast.velocity, full.velocity, "velocity tick {tick}");
        assert_eq!(fast.ground, full.ground, "ground tick {tick}");
        assert_eq!(
            (
                fast.collision.horizontal,
                fast.collision.vertical,
                fast.collision.below,
                fast.collision.minor,
            ),
            (
                full.collision.horizontal,
                full.collision.vertical,
                full.collision.below,
                full.collision.minor,
            ),
            "collision tick {tick}"
        );
        assert_eq!(fast.speed, full.speed, "speed tick {tick}");
        assert_eq!(fast.head_yaw, full.head_yaw, "head_yaw tick {tick}");
        assert_eq!(fast.fire_ticks, full.fire_ticks, "fire_ticks tick {tick}");
        assert_eq!(
            fast.on_climbable, full.on_climbable,
            "climbable tick {tick}"
        );
        assert_eq!(
            fast.fall_distance, full.fall_distance,
            "fall_distance tick {tick}"
        );
        assert_eq!(
            fast.health.tick_count, full.health.tick_count,
            "health ticks {tick}"
        );
        assert_eq!(fast.health.health(), full.health.health(), "health {tick}");
        // `Replay::play` clears `body.events` after each replayed tick and nothing reads
        // `MotionEvent::Landing`, so confirm no non-`Landing` events were skipped.
        assert!(
            full.events[full_before_events..]
                .iter()
                .all(|e| matches!(e, MotionEvent::Landing { .. })),
            "unexpected non-Landing events at tick {tick}: {:?}",
            &full.events[full_before_events..]
        );
    }

    // Bouncy / faster-than-gravity cases must fall back to full path and match.
    let mut fast_bounce = fast.clone();
    fast_bounce.velocity.y = -0.3;
    let key = fast_bounce.attributes.key("minecraft:bounciness").unwrap();
    let h = fast_bounce
        .attributes
        .get_instance(Some(key))
        .unwrap()
        .unwrap();
    fast_bounce
        .attributes
        .edit(&h, |a, o| a.set_base_value(0.8, o))
        .unwrap();
    let mut full_bounce = fast_bounce.clone();
    assert!(
        !mw!(&mut fast_bounce, &mut scene)
            .try_fast_stationary_replayed_tick(crate::sprint::Keys::default(), false)
            .unwrap(),
        "bouncy / -vy > gravity must not enter fast path"
    );
    mw!(&mut fast_bounce, &mut scene)
        .replayed_tick(crate::sprint::Keys::default(), false)
        .unwrap();
    full_bounce.replaying = true;
    mw!(&mut full_bounce, &mut scene_full)
        .local_tick(crate::sprint::Keys::default(), |world, _, _| {
            world.player.set_sprinting(false)
        })
        .unwrap();
    full_bounce.replaying = false;
    assert_eq!(fast_bounce.velocity, full_bounce.velocity);
    assert_eq!(fast_bounce.position, full_bounce.position);
}

#[test]
#[ignore = "daily"]
fn stationary_server_travel_fast_path_matches_full_and_respects_bounciness() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let build_scene = || {
        let mut s = World::test_scene(assets, render, 2).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                s.set(x, 63, z, stone).unwrap();
            }
        }
        s
    };
    let mut scene = build_scene();
    macro_rules! mw {
        ($player:expr, $scene:expr) => {
            MotionWorld {
                tag_items: None,
                block_registries: None,
                loop_context: crate::test_resources::server_loop_context(),
                damage_hooks: &crate::health::DamageHooks::NONE,
                entities: &[],
                player: $player,
                scene: $scene,
                assets,
                collision_binding: binding,
                resources,
                health: &resources.health,
            }
        };
    }
    let mut seed = PlayerMotion::new(resources, Vec3::new(20.5, 64.0, 20.5)).unwrap();
    for _ in 0..2 {
        mw!(&mut seed, &mut scene)
            .local_tick(crate::sprint::Keys::default(), |world, _, _| {
                world.player.set_sprinting(false)
            })
            .unwrap();
    }
    mw!(&mut seed, &mut scene).server_travel().unwrap();

    let mut fast = seed.clone();
    let mut full = seed;
    let mut scene_full = build_scene();
    for tick in 0..5 {
        assert!(
            mw!(&mut fast, &mut scene)
                .can_fast_ground_server_travel()
                .unwrap()
                .is_some(),
            "tick {tick} must qualify for fast server_travel"
        );
        mw!(&mut fast, &mut scene).server_travel().unwrap();

        // Exact baseline fallback path from before this change.
        full.drop_tiny_velocity();
        let (position, ground, collision) = (full.position, full.ground, full.collision);
        let (history, fall_distance, steps) =
            (full.history.clone(), full.fall_distance, full.steps);
        let events = full.events.len();
        full.server_copy = true;
        mw!(&mut full, &mut scene_full)
            .player_travel(Vec3::ZERO)
            .unwrap();
        full.server_copy = false;
        (full.position, full.ground, full.collision) = (position, ground, collision);
        (full.history, full.fall_distance, full.steps) = (history, fall_distance, steps);
        full.events.truncate(events);

        assert_eq!(
            fast.velocity, full.velocity,
            "server_travel velocity tick {tick}"
        );
        assert_eq!(
            fast.position, full.position,
            "server_travel position tick {tick}"
        );
        assert_eq!(fast.ground, full.ground, "server_travel ground tick {tick}");
    }

    // Blocker #1 regression check: -vy > gravity with bounciness > 0 must fall back and bounce.
    let mut bounce = fast.clone();
    bounce.velocity = Vec3::new(0.0, -0.3, 0.0);
    let key = bounce.attributes.key("minecraft:bounciness").unwrap();
    let h = bounce.attributes.get_instance(Some(key)).unwrap().unwrap();
    bounce
        .attributes
        .edit(&h, |a, o| a.set_base_value(0.8, o))
        .unwrap();
    let mut bounce_full = bounce.clone();
    assert!(
        mw!(&mut bounce, &mut scene)
            .can_fast_ground_server_travel()
            .unwrap()
            .is_none(),
        "-vy > gravity must fall back so entity bounciness runs"
    );
    mw!(&mut bounce, &mut scene).server_travel().unwrap();

    bounce_full.drop_tiny_velocity();
    bounce_full.server_copy = true;
    mw!(&mut bounce_full, &mut scene_full)
        .player_travel(Vec3::ZERO)
        .unwrap();
    bounce_full.server_copy = false;
    assert!(
        bounce.velocity.y > 0.0,
        "player must bounce vertically: {:?}",
        bounce.velocity
    );
    assert_eq!(bounce.velocity, bounce_full.velocity);
}

#[test]
#[ignore = "daily"]
fn dry_air_fluid_base_tick_fast_path_matches_full() {
    let Some(v) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (assets, render, binding, resources) = (&v.assets, &v.render, &v.collision, &v.motion);
    let mut scene = World::test_scene(assets, render, 2).unwrap();
    let mut seed = PlayerMotion::new(resources, Vec3::new(20.5, 64.0, 20.5)).unwrap();
    let mut full = seed.clone();
    // First tick still runs full path because `fluids.first_tick` is true.
    let mut w_seed = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut seed,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    assert!(!w_seed.is_dry_air_fast());
    w_seed.fluid_base_tick().unwrap();
    assert!(w_seed.is_dry_air_fast());
    for tick in 0..5 {
        assert!(
            w_seed.is_dry_air_fast(),
            "dry air tick {tick} must take fast path"
        );
        w_seed.fluid_base_tick().unwrap();
    }
    // Baseline full path (without dry-air fast skip): clear `first_tick` once then run full body.
    let mut w_full = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut full,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    for _ in 0..6 {
        w_full.player.fluids.was_eye_in_water = w_full.player.is_eye_in_fluid(fluids::WATER);
        w_full.update_fluids().unwrap();
        w_full.tick_remaining_fire().unwrap();
        w_full.player.fluids.first_tick = false;
    }
    assert_eq!(
        seed.fluids.was_touching_water,
        full.fluids.was_touching_water
    );
    assert_eq!(seed.fluids.was_eye_in_water, full.fluids.was_eye_in_water);
    assert_eq!(seed.swimming, full.swimming);
    assert_eq!(seed.fire_ticks, full.fire_ticks);
    assert_eq!(seed.fall_distance, full.fall_distance);
    assert_eq!(seed.events.len(), full.events.len());

    // Placing water or lava inside the box or at a custom eye height forces fallback.
    let water = assets.registry.default_state("minecraft:water").unwrap();
    let lava = assets.registry.default_state("minecraft:lava").unwrap();
    let air = assets.registry.default_state("minecraft:air").unwrap();
    scene.set(20, 64, 20, water).unwrap();
    let w_check = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut seed,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    assert!(!w_check.is_dry_air_fast());
    scene.set(20, 64, 20, lava).unwrap();
    let w_lava = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut seed,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    assert!(!w_lava.is_dry_air_fast());
    scene.set(20, 64, 20, air).unwrap();
    scene.set(20, 67, 20, water).unwrap();
    seed.dimensions.eye_height = 3.5;
    let w_eye = MotionWorld {
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut seed,
        scene: &mut scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    assert!(!w_eye.is_dry_air_fast());
}
