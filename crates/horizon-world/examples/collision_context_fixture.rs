use horizon_core::{
    geometry::{Aabb, Axis, Vec3, VoxelShape},
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
fn values(
    out: &mut impl Write,
    label: &str,
    values: impl IntoIterator<Item = f64>,
) -> io::Result<()> {
    write!(out, "{label}")?;
    for v in values {
        let bits = if v.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            v.to_bits()
        };
        write!(out, " {bits:016x}")?;
    }
    writeln!(out)
}
fn box_values(out: &mut impl Write, label: &str, b: Aabb) -> io::Result<()> {
    values(
        out,
        label,
        [
            b.min(Axis::X),
            b.min(Axis::Y),
            b.min(Axis::Z),
            b.max(Axis::X),
            b.max(Axis::Y),
            b.max(Axis::Z),
        ],
    )
}
fn emit(out: &mut impl Write, shape: &VoxelShape) -> io::Result<()> {
    writeln!(
        out,
        "shape {} {} {}",
        shape.is_empty(),
        shape.boxes().len(),
        shape.is_cube()
    )?;
    for (axis, name) in Axis::ALL.into_iter().zip(["x", "y", "z"]) {
        values(out, name, shape.coordinates(axis).iter().copied())?;
    }
    let axes = shape.cube_coordinate_axes();
    writeln!(
        out,
        "axes {}",
        u8::from(axes[0]) + 2 * u8::from(axes[1]) + 4 * u8::from(axes[2])
    )?;
    values(
        out,
        "extrema",
        Axis::ALL
            .map(|a| shape.min(a))
            .into_iter()
            .chain(Axis::ALL.map(|a| shape.max(a))),
    )?;
    match shape.bounds() {
        Some(b) => box_values(out, "bounds", b)?,
        None => writeln!(out, "bounds none")?,
    }
    for b in shape.boxes() {
        box_values(out, "box", *b)?;
    }
    Ok(())
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
        if let Some(config) = f.get("scaffolding") {
            let state = horizon_world::chunk::BlockState {
                name: "minecraft:scaffolding".into(),
                properties: [
                    ("distance".into(), config[0].as_i64().unwrap().to_string()),
                    ("bottom".into(), config[1].as_bool().unwrap().to_string()),
                    (
                        "waterlogged".into(),
                        config[2].as_bool().unwrap().to_string(),
                    ),
                ]
                .into(),
            };
            let rule =
                horizon_world::block_collision::ScaffoldingCollision::from_state(&state)?.unwrap();
            emit(
                &mut out,
                &rule.shape(
                    &context,
                    BlockPos::new(17, config[3].as_i64().unwrap() as i32, -33),
                ),
            )?;
            continue;
        }
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
