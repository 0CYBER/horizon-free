use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::BlockPos,
};
use horizon_world::{
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    entity_obstruction::EntityIdentity,
    fluid::{FluidKind, FluidState},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
const ITEMS: [&str; 5] = [
    "minecraft:air",
    "minecraft:stone",
    "minecraft:scaffolding",
    "minecraft:powder_snow_bucket",
    "minecraft:leather_boots",
];
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let fluids = FluidKind::ALL
        .map(|kind| FluidState::from_json(&format!("{{\"id\":\"{}\"}}", kind.name())).unwrap());
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let player = CollisionEntity {
            identity: EntityIdentity(1),
            y: number(&f["y"]),
            descending: f["descending"].as_bool().unwrap(),
            main_hand: Some(CollisionItem {
                item: ITEMS[f["item"].as_u64().unwrap() as usize],
                count: f["count"].as_i64().unwrap() as i32,
            }),
        };
        let marker = CollisionEntity {
            identity: EntityIdentity(2),
            main_hand: None,
            ..player
        };
        let always = f["always"].as_bool().unwrap();
        let other = number(&f["other"]);
        let context = match f["mode"].as_str().unwrap() {
            "empty" => CollisionContext::empty(always),
            "position" => CollisionContext::position(player.y),
            "player" => CollisionContext::for_entity(player, always),
            "marker" => CollisionContext::for_entity(marker, always),
            "factory_player" => CollisionContext::for_entity(player, false),
            "factory_marker" => CollisionContext::for_entity(marker, false),
            "placement" => CollisionContext::placement(Some(player)),
            "placement_null" => CollisionContext::placement(None),
            "with_player" => CollisionContext::with_position(Some(player), other),
            "with_marker" => CollisionContext::with_position(Some(marker), other),
            "with_null" => CollisionContext::with_position(None, other),
            _ => unreachable!(),
        };
        let held = ITEMS.iter().enumerate().fold(0, |mask, (i, item)| {
            mask | (u32::from(context.is_holding_item(item)) << i)
        });
        writeln!(
            out,
            "context {} {} {} {} {}",
            context.is_descending(),
            context.is_placement(),
            context.always_collide_with_fluid(),
            context.entity().map_or(0, |id| id.0),
            held
        )?;
        write!(out, "above ")?;
        for query in f["above"].as_array().unwrap() {
            let top = number(&query[1]);
            let shape = if query[2].as_bool().unwrap() {
                VoxelShape::empty()
            } else {
                VoxelShape::from_box(Aabb::new(
                    Vec3::new(0.0, top - 0.25, 0.0),
                    Vec3::new(1.0, top, 1.0),
                ))?
            };
            for fallback in [false, true] {
                write!(
                    out,
                    "{}",
                    u8::from(context.is_above(
                        &shape,
                        BlockPos::new(0, query[0].as_i64().unwrap() as i32, 0),
                        fallback
                    ))
                )?;
            }
        }
        writeln!(out)?;
        for first in fluids {
            for second in fluids {
                let mut calls = 0;
                let mut last = -1;
                let result = context.can_stand_on_fluid(first, second, |entity, fluid| {
                    assert_eq!(entity, player.identity);
                    calls += 1;
                    last = fluid.kind() as i32;
                    if f["fail"].as_bool().unwrap() {
                        Err(())
                    } else {
                        Ok(f["stand"].as_bool().unwrap())
                    }
                });
                let result = result.map_or_else(|()| "error".into(), |v| v.to_string());
                writeln!(out, "fluid {result} {calls} {last}")?;
            }
        }
    }
    Ok(())
}
