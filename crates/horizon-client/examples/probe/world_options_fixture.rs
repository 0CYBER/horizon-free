//! Packet payload oracle companion for tools/test_world_options_parity.py.
use horizon_protocol::{buf::Reader, packets::play::*};
use std::{error::Error, fmt::Write};

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = crate::args().collect();
    let mut output = String::new();
    for line in std::fs::read_to_string(&args[1])?.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let entries = if fields.len() == 3 {
            vec![GameRuleEntry {
                name: format!("minecraft:{}", fields[1]),
                value: fields[2].into(),
            }]
        } else {
            vec![]
        };
        let bytes = match fields[0] {
            "difficulty" => Serverbound::ChangeDifficulty(ChangeDifficultyRequest {
                difficulty: fields[1].parse()?,
            })
            .encode(),
            "lock" => Serverbound::LockDifficulty(LockDifficulty {
                locked: fields[1].parse()?,
            })
            .encode(),
            "set" => Serverbound::SetGameRule(SetGameRule { entries }).encode(),
            "values" => Clientbound::GameRuleValues(GameRuleValues { entries }).encode(),
            _ => return Err("unknown case".into()),
        };
        let mut reader = Reader::new(&bytes);
        reader.varint()?;
        for byte in reader.rest() {
            write!(output, "{byte:02x}")?;
        }
        output.push('\n');
    }
    std::fs::write(&args[2], output)?;
    Ok(())
}
