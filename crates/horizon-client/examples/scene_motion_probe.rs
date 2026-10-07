//! Repeated movement over the same owned actor and resident scene as the client.
#![forbid(unsafe_code)]
use horizon_client::{
    Result,
    assets::Assets,
    collisions,
    motion::{MotionResources, MotionWorld, PlayerMotion},
    world::TestWorld,
};
use horizon_core::geometry::Vec3;
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
        "{} {} {} {} {} {} {} {:016x} {} {} {} {:08x}",
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
        player.speed.to_bits()
    )?;
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
    let scene = TestWorld::new(&assets, 1)?;
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
        let mut player = PlayerMotion::new(&resources, vector(&f["position"]))?;
        for op in f["ops"].as_array().unwrap() {
            let mut world = MotionWorld {
                player: &mut player,
                scene: &scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            match op["mode"].as_str().unwrap() {
                "travel" => world.travel_in_air(vector(&op["value"]))?,
                "move" => world.move_self(vector(&op["value"]))?,
                "jump" => world.jump_from_ground()?,
                "sprint" => world.player.set_sprinting(op["value"].as_bool().unwrap())?,
                "speed" => world.player.speed = single(&op["value"]),
                "yaw" => world.player.yaw = single(&op["value"]),
                "velocity" => world.player.velocity = vector(&op["value"]),
                "stuck" => world.player.stuck_multiplier = vector(&op["value"]),
                "fall" => world.player.fall_distance = number(&op["value"]),
                "jumping" => world.player.jumping = op["value"].as_bool().unwrap(),
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
            snapshot(&mut out, &player)?;
            player.events.clear();
        }
        writeln!(out, "end")?;
    }
    Ok(())
}
