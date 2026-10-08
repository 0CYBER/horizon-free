//! Headless transport exercising the actual client resource loader and chunks.
#![forbid(unsafe_code)]
use horizon_client::{assets, collisions, world};
use horizon_core::geometry::{Aabb, Vec3};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, Write},
    path::PathBuf,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn vector(value: &Value) -> Result<Vec3> {
    let a = value
        .as_array()
        .filter(|v| v.len() == 3)
        .ok_or("expected vector")?;
    let number = |i: usize| -> Result<f64> {
        Ok(f64::from_bits(u64::from_str_radix(
            a[i].as_str().ok_or("expected double bits")?,
            16,
        )?))
    };
    Ok(Vec3::new(number(0)?, number(1)?, number(2)?))
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: scene_collision_probe RESOURCES FIXTURE_DIRECTORY RADIUS".into());
    }
    let resources = PathBuf::from(&args[0]);
    let dir = PathBuf::from(&args[1]);
    let radius: i32 = args[2].parse()?;
    if !(1..=16).contains(&radius) {
        return Err("radius must be 1..16".into());
    }
    let assets = assets::Assets::load(&resources)?;
    let world = world::World::test_scene(&assets, radius)?;
    let binding = collisions::load(&resources, assets.registry)?;
    let mut scene = collisions::SceneCollisions {
        world: &world,
        binding: &binding,
    };
    fs::write(
        dir.join("scene.json"),
        serde_json::to_vec(&serde_json::json!({
            "radius":radius,"resident_chunks":world.chunks.len(),"model_gallery":assets.model_gallery,
        }))?,
    )?;
    let mut output = std::io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in std::io::BufReader::new(fs::File::open(dir.join("inputs.txt"))?).lines() {
        let f: Value = serde_json::from_str(&line?)?;
        let bounds = Aabb::new(vector(&f["min"])?, vector(&f["max"])?);
        let delta = vector(&f["delta"])?;
        let step = f32::from_bits(u32::from_str_radix(
            f["step"].as_str().ok_or("expected step bits")?,
            16,
        )?);
        let movement = scene.resolve(
            bounds,
            delta,
            step,
            f["grounded"].as_bool().ok_or("expected grounded flag")?,
        )?;
        writeln!(
            output,
            "move {:016x} {:016x} {:016x} clear {} {}",
            movement.x.to_bits(),
            movement.y.to_bits(),
            movement.z.to_bits(),
            scene.unobstructed(bounds)?,
            scene.unobstructed(bounds.move_by(movement))?
        )?;
    }
    Ok(())
}
