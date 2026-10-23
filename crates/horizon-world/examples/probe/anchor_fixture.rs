//! Original comparison transport for generation bounds and vertical anchors.
use horizon_world::worldgen::anchor::{VerticalAnchor, WorldGenerationContext};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn anchor(
    a: VerticalAnchor,
    context: WorldGenerationContext,
    out: &mut impl Write,
) -> io::Result<()> {
    writeln!(
        out,
        "anchor {} {}",
        a.resolve(context),
        a.to_json().unwrap_or_else(|_| "error".to_owned())
    )
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let [min, height, sea] = std::array::from_fn(|i| f["context"][i].as_i64().unwrap() as i32);
        let context = if let Some(g) = f.get("generator") {
            WorldGenerationContext::with_generator(
                min,
                height,
                g[0].as_i64().unwrap() as i32,
                g[1].as_i64().unwrap() as i32,
                g[2].as_i64().unwrap() as i32,
            )
        } else {
            WorldGenerationContext::new(min, height, sea)
        };
        writeln!(
            out,
            "context {} {} {}",
            context.min_y, context.height, context.sea_level
        )?;
        for a in [
            VerticalAnchor::BOTTOM,
            VerticalAnchor::TOP,
            VerticalAnchor::SEA_LEVEL,
        ] {
            anchor(a, context, &mut out)?;
        }
        for e in f["values"].as_array().unwrap() {
            let v = e.as_i64().unwrap() as i32;
            for a in [
                VerticalAnchor::Absolute(v),
                VerticalAnchor::AboveBottom(v),
                VerticalAnchor::BelowTop(v),
                VerticalAnchor::RelativeToSeaLevel(v),
            ] {
                anchor(a, context, &mut out)?;
            }
        }
        for e in f["json"].as_array().unwrap() {
            match VerticalAnchor::from_json(e.as_str().unwrap()) {
                Ok(a) => anchor(a, context, &mut out)?,
                Err(_) => writeln!(out, "error")?,
            }
        }
    }
    out.flush()?;
    Ok(())
}
