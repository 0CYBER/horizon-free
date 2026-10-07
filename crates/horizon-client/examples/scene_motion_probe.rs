//! Repeated movement over the same owned actor and resident scene as the client.
#![forbid(unsafe_code)]
use horizon_client::{
    Result,
    assets::Assets,
    collisions,
    motion::{MotionEvent, MotionResources, MotionWorld, PlayerMotion},
    world::TestWorld,
};
use horizon_core::geometry::Vec3;
use horizon_core::{position::BlockPos, random::RandomSource};
use horizon_world::attributes::{Modifier, Operation};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, Write},
    path::PathBuf,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn position(p: Option<horizon_core::position::BlockPos>) -> String {
    p.map_or_else(|| "none".into(), |p| format!("{},{},{}", p.x, p.y, p.z))
}
fn snapshot(out: &mut impl Write, player: &PlayerMotion) -> Result<()> {
    // Compare membership and stored values without allocating lazy instances or
    // evaluating computed values. Runtime identity hashes differ between VMs.
    let mut attributes = Vec::new();
    for (handle, key, value) in player.attributes.instances() {
        let mut modifiers: Vec<_> = value
            .modifiers()
            .iter()
            .map(|m| {
                format!(
                    "{}:{:016x}:{}:{}",
                    m.id(),
                    m.amount().to_bits(),
                    m.operation() as u8,
                    value.permanent_modifiers().iter().any(|p| p.id() == m.id())
                )
            })
            .collect();
        modifiers.sort();
        attributes.push(format!(
            "{}:{:016x}:{}:{:016x}:{}:{}:[{}]",
            key.name(),
            value.base_value().to_bits(),
            value.is_dirty(),
            value.cached_value().to_bits(),
            player.attributes.updates().contains(Some(handle)),
            player.attributes.sync().contains(Some(handle)),
            modifiers.join(",")
        ));
    }
    attributes.sort();
    for v in [
        player.position,
        player.velocity,
        player.bounds().min_position(),
        player.bounds().max_position(),
    ] {
        write!(
            out,
            "{:016x} {:016x} {:016x} ",
            v.x.to_bits(),
            v.y.to_bits(),
            v.z.to_bits()
        )?;
    }
    writeln!(
        out,
        "{} {} {} {} {} {} {} {:016x} {} {} {} {:08x} {} {:08x} {} {} {} | {}",
        player.collision.horizontal,
        player.collision.vertical,
        player.collision.below,
        player.collision.minor,
        player.ground.on_ground,
        player.ground.on_ground_no_blocks,
        position(player.ground.supporting_block),
        player.fall_distance.to_bits(),
        position(player.last_climbable),
        player.needs_sync,
        player.sync_position,
        player.speed.to_bits(),
        player.no_jump_delay,
        player.head_yaw.to_bits(),
        player.jump_trigger_time,
        player.impulse.reset_grace_time,
        player.impulse.impact_position.map_or_else(
            || "none".into(),
            |p| format!(
                "{:016x},{:016x},{:016x}",
                p.x.to_bits(),
                p.y.to_bits(),
                p.z.to_bits()
            )
        ),
        attributes.join(";")
    )?;
    let events: Vec<_> = player
        .events
        .iter()
        .filter_map(|e| match e {
            MotionEvent::Fall(e) => Some(e.trace()),
            MotionEvent::Health(e) => Some(e.trace()),
            _ => None,
        })
        .collect();
    writeln!(out, "events {}", events.join(" | "))?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: scene_motion_probe RESOURCES FIXTURE_DIRECTORY".into());
    }
    let root = PathBuf::from(&args[0]);
    let dir = PathBuf::from(&args[1]);
    let assets = Assets::load(&root)?;
    let mut scene = TestWorld::new(&assets, 1)?;
    let binding = collisions::load(&root, &assets.registry)?;
    let resources = MotionResources::load(&root, &assets)?;
    fs::write(
        dir.join("scene.json"),
        serde_json::to_vec(
            &serde_json::json!({"radius":1,"resident_chunks":scene.chunks.len(),"model_gallery":assets.model_gallery}),
        )?,
    )?;
    let mut out = std::io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in std::io::BufReader::new(fs::File::open(dir.join("inputs.txt"))?).lines() {
        let f: Value = serde_json::from_str(&line?)?;
        scene.health_rules = horizon_client::health::HealthRules::default();
        let mut player = PlayerMotion::new(&resources, vector(&f["position"]))?;
        for op in f["ops"].as_array().unwrap() {
            let mut world = MotionWorld {
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            match op["mode"].as_str().unwrap() {
                "tick_count" => {
                    world.player.health.tick_count = op["value"].as_i64().unwrap() as i32
                }
                "rules" => {
                    world.scene.health_rules = horizon_client::health::HealthRules {
                        difficulty: [
                            horizon_world::food::Difficulty::Peaceful,
                            horizon_world::food::Difficulty::Easy,
                            horizon_world::food::Difficulty::Normal,
                            horizon_world::food::Difficulty::Hard,
                        ][op["difficulty"].as_u64().unwrap() as usize],
                        natural_regeneration: op["regenerate"].as_bool().unwrap(),
                    };
                }
                "food" => {
                    world.player.health.food = horizon_world::food::FoodData {
                        food: op["food"].as_i64().unwrap() as i32,
                        saturation: single(&op["saturation"]),
                        exhaustion: single(&op["exhaustion"]),
                        timer: op["timer"].as_i64().unwrap() as i32,
                    };
                }
                "exhaust" => world
                    .player
                    .health
                    .food
                    .add_exhaustion(single(&op["value"])),
                "eat" => world.player.health.food.eat(
                    op["nutrition"].as_i64().unwrap() as i32,
                    single(&op["value"]),
                ),
                "food_tick" => world
                    .player
                    .food_tick(&resources.health, world.scene.health_rules)?,
                "peaceful" => world
                    .player
                    .peaceful_regeneration(world.scene.health_rules)?,
                "simulation_tick" => {
                    let v = vector(&op["value"]);
                    world.simulation_tick(
                        [v.x as f32, v.z as f32],
                        op["jumping"].as_bool().unwrap(),
                    )?;
                }
                "clock" => world.player.health.advance_clock(),
                "health" | "absorption" => world
                    .player
                    .set_health(single(&op["value"]), op["mode"] == "absorption")?,
                "hurt" => {
                    let source = resources
                        .health
                        .damage
                        .id(op["name"].as_str().unwrap())
                        .ok_or("missing fixture damage source")?;
                    let kind = match op["name"].as_str().unwrap() {
                        "minecraft:fall" => horizon_world::block_landing::LandingDamage::Fall,
                        "minecraft:stalagmite" => {
                            horizon_world::block_landing::LandingDamage::Stalagmite
                        }
                        _ => return Err("fixture source is not a landing source".into()),
                    };
                    let amount = single(&op["value"]);
                    world.player.events.push(MotionEvent::Fall(
                        horizon_client::landing::FallEvent::Damage {
                            source: kind,
                            amount,
                        },
                    ));
                    world.player.hurt(
                        &resources.health,
                        world.scene.health_rules,
                        source,
                        amount,
                    )?;
                }
                "seed" => world.scene.random.set_seed(op["value"].as_i64().unwrap()),
                "impulse" => world.player.impulse.set_ignore_fall_damage(
                    op["ignore"].as_bool().unwrap(),
                    if op["value"].is_null() {
                        None
                    } else {
                        Some(vector(&op["value"]))
                    },
                ),
                "block" => {
                    let p = &op["position"];
                    let mut state = assets
                        .registry
                        .default_state(op["name"].as_str().unwrap())
                        .ok_or("missing fixture block")?;
                    if let Some(properties) = op["properties"].as_object() {
                        let mut definition = assets.registry.state(state).unwrap().clone();
                        for (name, value) in properties {
                            definition
                                .properties
                                .insert(name.clone(), value.as_str().unwrap().into());
                        }
                        state = assets
                            .registry
                            .id(&definition)
                            .ok_or("missing fixture state")?;
                    }
                    world.scene.set(
                        p[0].as_i64().unwrap() as i32,
                        p[1].as_i64().unwrap() as i32,
                        p[2].as_i64().unwrap() as i32,
                        state,
                    )?;
                }
                "landing" => {
                    let p = &op["position"];
                    let p = BlockPos::new(
                        p[0].as_i64().unwrap() as i32,
                        p[1].as_i64().unwrap() as i32,
                        p[2].as_i64().unwrap() as i32,
                    );
                    let state = world.scene.block(p.x, p.y, p.z);
                    horizon_world::entity_move::EntityMoveWorld::check_fall_damage(
                        &mut world,
                        number(&op["value"]),
                        op["ground"].as_bool().unwrap(),
                        state,
                        p,
                    )?;
                }
                "travel" => world.travel_in_air(vector(&op["value"]))?,
                "tick" => {
                    let v = vector(&op["value"]);
                    world.movement_tick([v.x as f32, v.y as f32, v.z as f32])?;
                }
                "player_tick" => {
                    let v = vector(&op["value"]);
                    world.player_movement_tick(
                        [v.x as f32, v.z as f32],
                        op["jumping"].as_bool().unwrap(),
                    )?;
                }
                "trigger" => world.player.jump_trigger_time = op["value"].as_i64().unwrap() as i32,
                "delay" => world.player.no_jump_delay = op["value"].as_i64().unwrap() as i32,
                "move" => world.move_self(vector(&op["value"]))?,
                "jump" => world.jump_from_ground()?,
                "sprint" => world.player.set_sprinting(op["value"].as_bool().unwrap())?,
                "speed" => world.player.speed = single(&op["value"]),
                "yaw" => world.player.yaw = single(&op["value"]),
                "velocity" => world.player.velocity = vector(&op["value"]),
                "stuck" => world.player.stuck_multiplier = vector(&op["value"]),
                "fall" => world.player.fall_distance = number(&op["value"]),
                "jumping" => world.player.jumping = op["value"].as_bool().unwrap(),
                "ack_attributes" => {
                    world.player.attributes.updates_mut().clear();
                    world.player.attributes.sync_mut().clear();
                }
                "modifier" => {
                    let key = world
                        .player
                        .attributes
                        .key(op["name"].as_str().unwrap())
                        .unwrap();
                    let handle = world
                        .player
                        .attributes
                        .get_instance(Some(key))
                        .map_err(|e| format!("{e:?}"))?
                        .unwrap();
                    let operation = match op["operation"].as_u64().unwrap() {
                        0 => Operation::AddValue,
                        1 => Operation::AddMultipliedBase,
                        2 => Operation::AddMultipliedTotal,
                        _ => return Err("unknown modifier operation".into()),
                    };
                    let modifier =
                        Modifier::new(op["id"].as_str().unwrap(), number(&op["value"]), operation)?;
                    world
                        .player
                        .attributes
                        .edit(&handle, |a, o| {
                            if op["permanent"].as_bool().unwrap() {
                                a.add_permanent(modifier, o)
                            } else {
                                a.add_transient(modifier, o)
                            }
                        })
                        .map_err(|e| format!("{e:?}"))?;
                }
                "attribute" => {
                    let key = world
                        .player
                        .attributes
                        .key(op["name"].as_str().unwrap())
                        .unwrap();
                    let handle = world
                        .player
                        .attributes
                        .get_instance(Some(key))
                        .map_err(|e| format!("{e:?}"))?
                        .unwrap();
                    world
                        .player
                        .attributes
                        .edit(&handle, |a, o| a.set_base_value(number(&op["value"]), o))
                        .map_err(|e| format!("{e:?}"))?;
                }
                _ => return Err("unknown motion operation".into()),
            }
            let check_block = scene.block(25, 3, 25);
            snapshot(&mut out, &player)?;
            writeln!(out, "block {}", check_block.0)?;
            let h = &player.health;
            let canonical = |v: f32| {
                if v.is_nan() {
                    f32::NAN.to_bits()
                } else {
                    v.to_bits()
                }
            };
            writeln!(
                out,
                "health {:08x} {:08x} {:08x} {} {} {:08x} {} {} {} {} {} {} {} {} {} {}",
                canonical(h.health()),
                canonical(h.absorption()),
                canonical(h.food.exhaustion),
                h.hurt.no_action_time,
                h.hurt.cooldown,
                canonical(h.hurt.last_hurt),
                h.hurt.hurt_duration,
                h.hurt.hurt_time,
                h.hurt.last_source.map_or("none", |id| resources
                    .health
                    .damage
                    .definition(id)
                    .unwrap()
                    .name
                    .as_str()),
                h.hurt.last_stamp,
                h.dead,
                h.sync_velocity,
                h.data.is_dirty(),
                h.data.item_dirty(h.ids.health).unwrap(),
                h.data.item_dirty(h.ids.absorption).unwrap(),
                h.game_time
            )?;
            writeln!(
                out,
                "food {} {:08x} {} {} {} {}",
                h.food.food,
                canonical(h.food.saturation),
                h.food.timer,
                h.tick_count,
                scene.health_rules.difficulty as u8,
                scene.health_rules.natural_regeneration
            )?;
            player.events.clear();
        }
        writeln!(out, "end")?;
    }
    Ok(())
}
