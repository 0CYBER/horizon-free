//! Original JSON block-state codec comparison transport.
use crate::support::common::tag;
use horizon_world::{
    DATA_VERSION,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId, BlockStateJsonCodec},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Write},
    path::Path,
};
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let facts: Value = serde_json::from_str(&fs::read_to_string(dir.join("facts-server.json"))?)?;
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", tag(&facts["states"])),
        ("defaults", tag(&facts["defaults"])),
    ]))?;
    let integers: BTreeSet<_> = facts["integers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v[0].as_str().unwrap().to_owned(),
                v[1].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let codec = BlockStateJsonCodec::new(&registry, integers)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for i in 0..registry.len() {
        let id = BlockStateId(i as u32);
        writeln!(
            out,
            "state {i} {} {}",
            codec.encode(id, false)?,
            codec.encode(id, true)?
        )?;
    }
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let input: Value = serde_json::from_str(line)?;
        match codec.decode(input[1].as_str().unwrap(), input[0].as_bool().unwrap()) {
            Err(_) => writeln!(out, "decode error")?,
            Ok(id) => writeln!(
                out,
                "decode {} {} {}",
                id.0,
                codec.encode(id, false)?,
                codec.encode(id, true)?
            )?,
        }
    }
    Ok(())
}
