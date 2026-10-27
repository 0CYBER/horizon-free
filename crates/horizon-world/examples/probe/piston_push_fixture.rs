//! The moving-piston push maths against the real game: `PistonMath.getMovementArea`,
//! `moveByPositionAndProgress`, `getMovement`, the `moveCollidedEntities` loop
//! over the block's own boxes, and `moveStuckEntities`' box with
//! `matchesStickyCritera`.
use crate::support::common::f64_hex;
use horizon_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
};
use horizon_world::{level::data::PushReaction, piston_push, piston_shape::MovingPistonState};
use serde_json::Value;
use std::{
    fs,
    io::{self, BufWriter, Write},
    path::Path,
};

fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

fn float(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

fn aabb(v: &Value) -> Aabb {
    let a = v.as_array().unwrap();
    Aabb::new(
        Vec3::new(number(&a[0]), number(&a[1]), number(&a[2])),
        Vec3::new(number(&a[3]), number(&a[4]), number(&a[5])),
    )
}

fn direction(index: i64) -> Direction {
    Direction::ALL[index.rem_euclid(6) as usize]
}

fn reaction(index: i64) -> PushReaction {
    match index.rem_euclid(5) {
        0 => PushReaction::PushPull,
        1 => PushReaction::Push,
        2 => PushReaction::Popped,
        3 => PushReaction::Immoveable,
        _ => PushReaction::IgnoreEntity,
    }
}

fn line(out: &mut impl Write, name: &str, b: Aabb) -> io::Result<()> {
    let (min, max) = (b.min_position(), b.max_position());
    writeln!(
        out,
        "{name} {} {} {} {} {} {}",
        f64_hex(min.x),
        f64_hex(min.y),
        f64_hex(min.z),
        f64_hex(max.x),
        f64_hex(max.y),
        f64_hex(max.z)
    )
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for text in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(text)?;
        let p = &f["pos"];
        let pos = BlockPos::new(
            p[0].as_i64().unwrap() as i32,
            p[1].as_i64().unwrap() as i32,
            p[2].as_i64().unwrap() as i32,
        );
        let progress = float(&f["progress"]);
        let piston = MovingPistonState {
            moved_state: horizon_world::registry::BlockStateId(0),
            direction: direction(f["direction"].as_i64().unwrap()),
            extending: f["extending"].as_bool().unwrap(),
            source: f["source"].as_bool().unwrap(),
            progress,
            previous_progress: progress,
        };
        let new_progress = float(&f["new_progress"]);
        let delta_progress = f64::from(new_progress - progress);
        let movement = piston.movement_direction();
        let block_box = aabb(&f["block_box"]);
        let entity = aabb(&f["entity"]);
        let movement_box = aabb(&f["movement_box"]);

        writeln!(
            out,
            "extended {:08x}",
            piston_push::extended_progress(piston, progress).to_bits()
        )?;
        let origin = piston_push::move_by_position_and_progress(pos, block_box, piston);
        line(&mut out, "origin", origin)?;
        line(
            &mut out,
            "area",
            piston_push::movement_area(block_box, movement, delta_progress),
        )?;
        for (name, face) in [
            ("east", Direction::East),
            ("west", Direction::West),
            ("up", Direction::Up),
            ("down", Direction::Down),
            ("south", Direction::South),
            ("north", Direction::North),
        ] {
            writeln!(
                out,
                "movement {name} {}",
                f64_hex(piston_push::get_movement(movement_box, face, entity))
            )?;
        }

        // `moveCollidedEntities`' loop: the furthest box reaches into the
        // entity, capped at the progress delta, plus the push offset.
        let boxes: Vec<Aabb> = f["boxes"].as_array().unwrap().iter().map(aabb).collect();
        let mut delta: f64 = 0.0;
        let mut capped = false;
        for block in &boxes {
            let moving = piston_push::movement_area(
                piston_push::move_by_position_and_progress(pos, *block, piston),
                movement,
                delta_progress,
            );
            if moving.intersects(entity) {
                delta = delta.max(piston_push::get_movement(moving, movement, entity));
                if delta >= delta_progress {
                    capped = true;
                    break;
                }
            }
        }
        let collided = if delta <= 0.0 {
            0.0
        } else {
            delta.min(delta_progress) + piston_push::PUSH_OFFSET
        };
        writeln!(out, "collided {} {capped}", f64_hex(collided))?;
        line(
            &mut out,
            "search",
            piston_push::movement_search_area(origin, movement, delta_progress),
        )?;
        let sticky = piston_push::sticky_area(pos, piston, number(&f["sticky_top"]));
        line(&mut out, "sticky", sticky)?;
        writeln!(
            out,
            "matches {}",
            piston_push::matches_sticky_criteria(
                sticky,
                reaction(f["reaction"].as_i64().unwrap()),
                f["on_ground"].as_bool().unwrap(),
                f["supported"].as_bool().unwrap(),
                Vec3::new(number(&f["entity_x"]), 0.0, number(&f["entity_z"])),
            )
        )?;
    }
    out.flush()?;
    Ok(())
}
