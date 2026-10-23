#![forbid(unsafe_code)]
use horizon_client::{
    Result,
    sprint::{Conditions, Keys, SprintState},
};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};
fn keys(mask: u64) -> Keys {
    Keys {
        forward: mask & 1 != 0,
        backward: mask & 2 != 0,
        left: mask & 4 != 0,
        right: mask & 8 != 0,
        jump: mask & 16 != 0,
        shift: mask & 32 != 0,
        sprint: mask & 64 != 0,
        analog: None,
    }
}
pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().skip(1).collect();
    let mut out = BufWriter::new(File::create(&args[1])?);
    for line in BufReader::new(File::open(&args[0])?).lines() {
        let f: Value = serde_json::from_str(&line?)?;
        let initial = keys(f["initial_keys"].as_u64().unwrap());
        let mut state = SprintState {
            trigger_time: f["trigger"].as_i64().unwrap() as i32,
            previous_forward: initial.direction()[1] > 1.0e-5,
            previous_shift: initial.shift,
        };
        let mut sprinting = f["sprint"].as_bool().unwrap();
        for step in f["steps"].as_array().unwrap() {
            let mask = step["flags"].as_u64().unwrap();
            let flag = |bit: u32| mask & (1u64 << bit) != 0u64;
            let conditions = Conditions {
                sprinting,
                mobility_restricted: flag(0),
                passenger: flag(1),
                vehicle_can_sprint: flag(2),
                vehicle_local: flag(3),
                enough_food: step["food"].as_i64().unwrap() as i32 as f32 > 6.0,
                may_fly: flag(4),
                flying: flag(5),
                shallow_water: flag(6),
                slow_item: flag(7) && !flag(8),
                fall_flying: flag(9),
                under_water: flag(10),
                moving_slowly: flag(11),
                swimming: flag(12),
                in_water: flag(13),
                on_ground: flag(14),
                horizontal_collision: flag(15),
                minor_horizontal_collision: flag(16),
            };
            let input = keys(step["keys"].as_u64().unwrap());
            let mut calls = Vec::new();
            state.tick(
                input,
                conditions,
                step["window"].as_i64().unwrap() as i32,
                |value| {
                    sprinting = value;
                    calls.push(value.to_string());
                    Ok::<_, std::convert::Infallible>(())
                },
            )?;
            let direction = input.direction();
            writeln!(
                out,
                "{} {} {:08x} {:08x} {} | {}",
                sprinting,
                state.trigger_time,
                direction[0].to_bits(),
                direction[1].to_bits(),
                state.previous_shift,
                calls.join(",")
            )?;
        }
        writeln!(out, "end")?;
    }
    Ok(())
}
