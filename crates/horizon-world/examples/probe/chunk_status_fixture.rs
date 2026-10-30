use horizon_world::{chunk_status::ChunkStatus, nbt::Tag, worldgen::terrain::PreLightChunkStage};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
};
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let mut out = io::BufWriter::new(fs::File::create(&args[2])?);
    writeln!(
        out,
        "structure_distance {}",
        ChunkStatus::MAX_STRUCTURE_DISTANCE
    )?;
    for status in ChunkStatus::ALL {
        let (name, maps) = if let Some(stage) = PreLightChunkStage::ALL.get(status.index()) {
            (format!("minecraft:{}", stage.name()), stage.heightmaps())
        } else {
            (status.name().to_owned(), status.heightmaps())
        };
        writeln!(
            out,
            "status {} {} {} {} {}",
            status.index(),
            name,
            status.parent().name(),
            status.is_level_chunk(),
            maps.iter()
                .map(|kind| kind.name())
                .collect::<Vec<_>>()
                .join(",")
        )?;
        for other in ChunkStatus::ALL {
            writeln!(
                out,
                "order {} {} {} {} {}",
                status >= other,
                status > other,
                status <= other,
                status < other,
                status.max(other).name()
            )?;
        }
    }
    for line in fs::read_to_string(&args[1])?.lines() {
        let row: Value = serde_json::from_str(line)?;
        let tag = match row[0].as_str().unwrap() {
            "string" => Tag::String(row[1].as_str().unwrap().into()),
            "int" => Tag::Int(row[1].as_i64().unwrap() as i32),
            "list" => Tag::List(Vec::new()),
            "compound" => Tag::Compound(Default::default()),
            _ => panic!("unknown tag"),
        };
        if let Some(name) = row[1].as_str() {
            writeln!(out, "lookup {}", ChunkStatus::by_name(name).name())?;
        }
        match ChunkStatus::from_nbt(&tag) {
            Ok(status) => {
                let Tag::String(encoded) = status.to_nbt() else {
                    unreachable!()
                };
                writeln!(out, "decoded {} {}", status.index(), encoded.to_string()?)?;
            }
            Err(_) => writeln!(out, "decoded error")?,
        }
    }
    out.flush()?;
    Ok(())
}
