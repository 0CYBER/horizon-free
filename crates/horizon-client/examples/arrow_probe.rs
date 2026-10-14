//! Arrows over the client's resident scene (`arrows::tick`), for
//! tools/test_arrow_parity.py: one line of state and one of events per operation.
#![forbid(unsafe_code)]
use horizon_client::{
    Result,
    arrows::{self, ArrowEvent, ArrowParticle, ArrowScene, ArrowSound},
    assets::Assets,
    blocks::BlockFacts,
    collisions,
    motion::{LAVA, MotionResources, WATER},
    projectiles::{Projectile, ProjectileKind},
    world::World,
};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, Write},
    path::PathBuf,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn bits(v: Vec3) -> String {
    format!(
        "{:016x},{:016x},{:016x}",
        v.x.to_bits(),
        v.y.to_bits(),
        v.z.to_bits()
    )
}
fn seed(v: &Value) -> i64 {
    v.as_str().unwrap().parse().unwrap()
}
fn particle(k: ArrowParticle) -> &'static str {
    match k {
        ArrowParticle::Bubble => "bubble",
        ArrowParticle::Crit => "crit",
        ArrowParticle::Splash => "splash",
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: arrow_probe RESOURCES FIXTURE_DIRECTORY --describe|REFERENCE".into());
    }
    let root = PathBuf::from(&args[0]);
    let dir = PathBuf::from(&args[1]);
    let assets = Assets::load(&root)?;
    let mut scene = World::test_scene(&assets, 1)?;
    fs::write(
        dir.join("scene.json"),
        serde_json::to_vec(
            &serde_json::json!({"radius":1,"resident_chunks":scene.chunks.len(),"model_gallery":assets.model_gallery}),
        )?,
    )?;
    if args[2] == "--describe" {
        return Ok(());
    }
    let binding = collisions::load(&root, assets.registry)?;
    let motion = MotionResources::load(&root, &assets)?;
    let facts = BlockFacts::load(&root, assets.registry)?;
    let side = &args[2];
    let mut out = std::io::BufWriter::new(fs::File::create(dir.join(format!("rust-{side}.txt")))?);
    let mut level_random = LegacyRandom::new(0);
    for line in std::io::BufReader::new(fs::File::open(dir.join("inputs.txt"))?).lines() {
        let f: Value = serde_json::from_str(&line?)?;
        let kind = ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: f["crit"].as_bool().unwrap(),
            effect: None,
        };
        let mut arrow = Projectile::new(kind, vector(&f["position"]), None)
            .with_arrow_random(LegacyRandom::new(seed(&f["seed"])));
        arrow.velocity = vector(&f["velocity"]);
        arrow.arrow.as_mut().unwrap().no_physics = f["no_physics"].as_bool().unwrap();
        for op in f["ops"].as_array().unwrap() {
            let mut events = Vec::new();
            match op["mode"].as_str().unwrap() {
                "tick" => {
                    if !arrow.arrow.as_ref().unwrap().removed {
                        let no_rain = |_| false;
                        let view = ArrowScene {
                            world: &scene,
                            registry: assets.registry,
                            facts: &facts,
                            collision: &binding,
                            fluids: &motion.fluids,
                            raining: &no_rain,
                        };
                        arrows::tick(&mut arrow, &view, &[]);
                    }
                    for event in arrow.arrow.as_mut().unwrap().events.drain(..) {
                        match event {
                            ArrowEvent::Particle {
                                kind,
                                position,
                                velocity,
                            } => events.push(format!(
                                "particle {} {} {}",
                                particle(kind),
                                bits(position),
                                bits(velocity)
                            )),
                            ArrowEvent::Sound {
                                sound,
                                volume,
                                pitch,
                            } => events.push(format!(
                                "sound {} {:08x} {:08x}",
                                match sound {
                                    ArrowSound::HitGround => "hit",
                                    ArrowSound::Splash => "splash",
                                    ArrowSound::Extinguish => "extinguish",
                                },
                                volume.to_bits(),
                                pitch.to_bits()
                            )),
                            ArrowEvent::BubbleColumnParticles { pos } => {
                                for _ in 0..2 {
                                    let x = f64::from(pos.x) + level_random.next_double();
                                    let z = f64::from(pos.z) + level_random.next_double();
                                    let at = Vec3::new(x, f64::from(pos.y + 1), z);
                                    events.push(format!(
                                        "sent splash {} {} {:016x}",
                                        bits(at),
                                        bits(Vec3::ZERO),
                                        1.0f64.to_bits()
                                    ));
                                    let x = f64::from(pos.x) + level_random.next_double();
                                    let z = f64::from(pos.z) + level_random.next_double();
                                    let at = Vec3::new(x, f64::from(pos.y + 1), z);
                                    events.push(format!(
                                        "sent bubble {} {} {:016x}",
                                        bits(at),
                                        bits(Vec3::new(0.0, 0.01, 0.0)),
                                        0.2f64.to_bits()
                                    ));
                                }
                            }
                            ArrowEvent::SplashEvent => events.push("splash-event".into()),
                            ArrowEvent::ShootEvent => events.push("shoot-event".into()),
                        }
                    }
                }
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
                    scene.set(
                        p[0].as_i64().unwrap() as i32,
                        p[1].as_i64().unwrap() as i32,
                        p[2].as_i64().unwrap() as i32,
                        state,
                    )?;
                }
                "velocity" => arrow.velocity = vector(&op["value"]),
                "level_seed" => level_random.set_seed(seed(&op["value"])),
                other => return Err(format!("unknown operation {other}").into()),
            }
            let a = arrow.arrow.as_ref().unwrap();
            writeln!(
                out,
                "arrow {} {} {:08x} {:08x} {} {} {} {} {} {} {} {} {} {:016x} {:016x} {} {} {:016x}",
                bits(arrow.position),
                bits(arrow.velocity),
                arrow.pitch.to_bits(),
                arrow.yaw.to_bits(),
                a.in_ground,
                a.in_ground_time,
                a.life,
                a.shake,
                a.last_state.map_or("none".into(), |s| s.0.to_string()),
                a.fire_ticks,
                a.was_touching_water,
                a.was_eye_in_water,
                a.first_tick,
                a.fluids.fluid_height(&Tags, WATER).to_bits(),
                a.fluids.fluid_height(&Tags, LAVA).to_bits(),
                a.removed,
                arrow.critical(),
                a.fall_distance.to_bits()
            )?;
            writeln!(out, "events {}", events.join(" | "))?;
        }
        writeln!(out, "end")?;
    }
    Ok(())
}
struct Tags;
impl horizon_world::entity_fluid::FluidMembership for Tags {
    fn fluid_in_tag(
        &self,
        kind: horizon_world::fluid::FluidKind,
        tag: horizon_world::entity_fluid::FluidTagId,
    ) -> bool {
        use horizon_world::fluid::FluidKind;
        if tag == WATER {
            matches!(kind, FluidKind::Water | FluidKind::FlowingWater)
        } else {
            matches!(kind, FluidKind::Lava | FluidKind::FlowingLava)
        }
    }
}
