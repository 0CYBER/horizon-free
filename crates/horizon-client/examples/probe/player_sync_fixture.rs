use horizon_client::player_sync::{PositionSender, teleport_velocity};
use horizon_core::geometry::Vec3;
use horizon_protocol::packets::play::{PlayerPosition, Serverbound as S};
use serde_json::Value;
use std::io::{BufRead, Write};

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value.as_str().unwrap(), 16).unwrap())
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let input = std::io::BufReader::new(std::fs::File::open(&args[1])?);
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[2])?);
    for line in input.lines() {
        let row: Value = serde_json::from_str(&line?)?;
        if row["kind"] == "movement" {
            let mut sender = PositionSender::default();
            for op in row["ops"].as_array().unwrap() {
                let pos = std::array::from_fn(|i| number(&op[i]));
                let rot = (number(&op[3]) as f32, number(&op[4]) as f32);
                if let Some(packet) =
                    sender.update(pos, rot, op[5].as_bool().unwrap(), op[6].as_bool().unwrap())
                {
                    let name = match packet {
                        S::MovePlayerPos(_) => "pos",
                        S::MovePlayerPosRot(_) => "posrot",
                        S::MovePlayerRot(_) => "rot",
                        S::MovePlayerStatus(_) => "status",
                        _ => unreachable!(),
                    };
                    write!(out, "{name} ")?;
                    for byte in &packet.encode()[1..] {
                        write!(out, "{byte:02x}")?;
                    }
                    writeln!(out)?;
                } else {
                    writeln!(out, "-")?;
                }
            }
        } else {
            let vector = |key: &str| {
                Vec3::new(
                    number(&row[key][0]),
                    number(&row[key][1]),
                    number(&row[key][2]),
                )
            };
            let rotation = |key: &str| (number(&row[key][0]) as f32, number(&row[key][1]) as f32);
            let old = rotation("old");
            let new = rotation("new");
            let delta = vector("delta");
            let packet = PlayerPosition {
                teleport: 0,
                position: [0.0; 3],
                delta: [delta.x, delta.y, delta.z],
                yaw: new.0,
                pitch: new.1,
                relatives: row["flags"].as_i64().unwrap() as i32,
            };
            let v = teleport_velocity(vector("velocity"), old, new, &packet);
            writeln!(
                out,
                "{:016x} {:016x} {:016x}",
                v.x.to_bits(),
                v.y.to_bits(),
                v.z.to_bits()
            )?;
        }
    }
    Ok(())
}
