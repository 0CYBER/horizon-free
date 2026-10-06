use horizon_assets::{
    bake::{ElementMatrix, bake_model},
    model::*,
    resolve::ModelLibrary,
};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};
fn bits(value: f32) -> u32 {
    if value.is_nan() {
        0x7fc0_0000
    } else {
        value.to_bits()
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut output = BufWriter::new(File::create(&args[2])?);
    for (index, line) in input.lines().enumerate() {
        let row: Value = serde_json::from_str(&line?)?;
        let models = row["models"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, raw)| Ok((name.clone(), CuboidModel::from_json(raw.as_str().unwrap())?)))
            .collect::<Result<_, horizon_assets::DecodeError>>()?;
        let library = ModelLibrary::new(models)?;
        for root in row["roots"].as_array().unwrap() {
            let name = root.as_str().unwrap();
            let baked = library.resolve(name).and_then(|model| {
                let quads = bake_model(&model)?.into_iter().map(|q| json!({
                    "positions":q.positions.map(|v| v.map(bits)),"uv":q.uv.map(|v| v.map(bits)),
                    "facing":q.facing.name(),"cull":q.cull.map(Direction::name),"tint":q.tint_index,
                    "shade":q.shade_direction_override.map(Direction::name),"light":q.light_emission,
                    "material":{"sprite":q.material.sprite,"translucent":q.material.force_translucent}
                })).collect::<Vec<_>>();
                let matrices: Vec<_> = model.elements().iter().map(|e| e.as_ref().unwrap().rotation.as_ref().map(|r| ElementMatrix::from_rotation(r).rows.map(|row| row.map(bits)))).collect();
                Ok(json!({"quads":quads,"matrices":matrices}))
            });
            let value = match baked {
                Ok(baked) => json!({"case":index,"name":name,"baked":baked}),
                Err(_) => json!({"case":index,"name":name,"error":true}),
            };
            writeln!(output, "{value}")?;
        }
    }
    Ok(())
}
