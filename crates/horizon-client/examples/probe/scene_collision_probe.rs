//! Headless transport exercising the actual client resource loader and chunks.
#![forbid(unsafe_code)]
use horizon_client::{assets, collisions, world};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::{
    collision_context::CollisionContext,
    entity_obstruction::EntityIdentity,
    world_movement::{self, CollisionCheckRequest, MovementRequest},
};
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
pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().skip(1).collect();
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
    let render = assets.reload(&resources)?;
    let world = world::World::test_scene(&assets, &render, radius)?;
    let binding = collisions::load(&resources, assets.registry)?;
    let mut scene = collisions::SceneCollisions {
        world: &world,
        binding: &binding,
        subject: None,
    };
    fs::write(
        dir.join("scene.json"),
        serde_json::to_vec(&serde_json::json!({
            "radius":radius,"resident_chunks":world.chunks.len(),"model_gallery":render.model_gallery,
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
        let entities = f["entities"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| {
                let a = v.as_array().ok_or("expected entity box")?;
                Ok(Aabb::new(
                    vector(&Value::Array(a[..3].to_vec()))?,
                    vector(&Value::Array(a[3..].to_vec()))?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut selected = collisions::EntityCollisions {
            blocks: scene,
            entities: &entities,
            border: None,
        };
        let movement = world_movement::resolve_world_movement(
            &mut selected,
            MovementRequest {
                entity: EntityIdentity(0),
                context: CollisionContext::position(bounds.min_position().y),
                bounds,
                movement: delta,
                max_step: step,
                on_ground: f["grounded"].as_bool().ok_or("expected grounded flag")?,
                candidate_budget: 1_000_000,
            },
        )?;
        let mut unobstructed = |bounds: Aabb| {
            world_movement::no_world_collision(
                &mut selected,
                CollisionCheckRequest {
                    entity: EntityIdentity(0),
                    context: CollisionContext::position(bounds.min_position().y),
                    bounds,
                    candidate_budget: 1_000_000,
                },
            )
        };
        writeln!(
            output,
            "move {:016x} {:016x} {:016x} clear {} {}",
            movement.x.to_bits(),
            movement.y.to_bits(),
            movement.z.to_bits(),
            unobstructed(bounds)?,
            unobstructed(bounds.move_by(movement))?
        )?;
        scene = selected.blocks;
    }
    Ok(())
}
