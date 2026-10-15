//! Arrows over the client's resident scene (`arrows::tick`), for
//! tools/test_arrow_parity.py: one line of state and one of events per operation.
#![forbid(unsafe_code)]
use horizon_client::{
    Result,
    arrows::{
        self, ArrowEvent, ArrowParticle, ArrowScene, ArrowSound, BlockReaction, Pickup, Touch,
    },
    assets::Assets,
    blocks::BlockFacts,
    collisions,
    items::ItemStack,
    mobs::Who,
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
        // ProjectileWeaponItem.useAmmo's stack, intangible when no ammo was used.
        arrow.arrow.as_mut().unwrap().set_item(&arrows::drawn_ammo(
            &ItemStack::new("minecraft:arrow", 1),
            f["intangible"].as_bool().unwrap(),
        ));
        for op in f["ops"].as_array().unwrap() {
            let mut events = Vec::new();
            let mut touched = "-".to_string();
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
                            mob_griefing: true,
                            tnt_explodes: true,
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
                            ArrowEvent::Block { pos, reaction } => match reaction {
                                BlockReaction::PrimeTnt => {
                                    // TntBlock.prime: PrimedTnt draws its push from the
                                    // level random, then removeBlock(pos, false).
                                    let _ = level_random.next_double();
                                    let at = Vec3::new(
                                        f64::from(pos.x) + 0.5,
                                        f64::from(pos.y),
                                        f64::from(pos.z) + 0.5,
                                    );
                                    events.push(format!("spawn tnt {} 80", bits(at)));
                                    events.push(format!(
                                        "sound tnt_primed {:08x} {:08x}",
                                        1.0f32.to_bits(),
                                        1.0f32.to_bits()
                                    ));
                                    events.push("prime-event".into());
                                    let air =
                                        assets.registry.default_state("minecraft:air").unwrap();
                                    scene.set(pos.x, pos.y, pos.z, air)?;
                                    events.push(format!(
                                        "write {},{},{} {} 3",
                                        pos.x, pos.y, pos.z, air.0
                                    ));
                                }
                                BlockReaction::Light(lit) => {
                                    scene.set(pos.x, pos.y, pos.z, lit)?;
                                    events.push(format!(
                                        "write {},{},{} {} 11",
                                        pos.x, pos.y, pos.z, lit.0
                                    ));
                                }
                            },
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
                "ignite" => arrow
                    .arrow
                    .as_mut()
                    .unwrap()
                    .ignite_for_seconds(f32::from_bits(
                        u32::from_str_radix(op["value"].as_str().unwrap(), 16).unwrap(),
                    )),
                "level_seed" => level_random.set_seed(seed(&op["value"])),
                "owner" => arrow
                    .arrow
                    .as_mut()
                    .unwrap()
                    .set_owner(match op["value"].as_str() {
                        Some("player") => Some(Who::Player),
                        Some("mob") => Some(Who::Mob(1)),
                        _ => None,
                    }),
                "pickup" => {
                    arrow.arrow.as_mut().unwrap().pickup =
                        Pickup::from_byte(op["value"].as_i64().unwrap())
                }
                // The game's touch: the inventory takes the arrow unless full,
                // and a creative player takes it anyway.
                "touch" => {
                    let creative = op["creative"].as_bool().unwrap();
                    let full = op["inventory"] == "full";
                    let a = arrow.arrow.as_mut().unwrap();
                    let (taken, arrows) = match a.player_touch(creative) {
                        Touch::Ignore => (false, 0),
                        Touch::Take => (true, 0),
                        Touch::Give(_) => (!full || creative, u32::from(!full)),
                    };
                    if taken {
                        a.removed = true;
                    }
                    touched = format!("{taken} {arrows}");
                }
                other => return Err(format!("unknown operation {other}").into()),
            }
            let a = arrow.arrow.as_ref().unwrap();
            writeln!(
                out,
                "arrow {} {} {:08x} {:08x} {} {} {} {} {} {} {} {} {} {:016x} {:016x} {} {} {:016x} {}",
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
                a.fall_distance.to_bits(),
                bits(arrow.previous)
            )?;
            writeln!(out, "events {}", events.join(" | "))?;
            let a = arrow.arrow.as_ref().unwrap();
            let intangible = a
                .pickup_item
                .data()
                .and_then(|d| d.other(arrows::INTANGIBLE))
                .is_some();
            writeln!(out, "pickup {} {intangible} {touched}", a.pickup.byte())?;
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
