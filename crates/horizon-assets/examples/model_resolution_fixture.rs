mod support;
use horizon_assets::{model::*, resolve::ModelLibrary};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

fn material(value: &Material) -> Value {
    json!({"sprite":value.sprite,"translucent":value.force_translucent})
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
            let Ok(model) = library.resolve(name) else {
                writeln!(output, "{}", json!({"case":index,"name":name,"error":true}))?;
                continue;
            };
            let elements: Vec<_> = model
                .elements()
                .iter()
                .map(|e| e.as_ref().map(support::element))
                .collect();
            let display: BTreeMap<_, _> = DISPLAY_CONTEXTS
                .into_iter()
                .zip(model.display.contexts.iter().map(support::transform))
                .collect();
            let textures: BTreeMap<_, _> = model
                .textures
                .iter()
                .map(|(key, value)| (key, material(value)))
                .collect();
            let queries: Vec<_> = row["queries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|raw| {
                    let slot = raw.as_str().unwrap();
                    match model.material(slot) {
                        Ok(value) => json!({"slot":slot,"value":value.map(material)}),
                        Err(_) => json!({"slot":slot,"error":true}),
                    }
                })
                .collect();
            writeln!(
                output,
                "{}",
                json!({"case":index,"name":name,"ao":model.ambient_occlusion,
                "gui":match model.gui_light { GuiLight::Front => "front", GuiLight::Side => "side" },
                "elements":elements,"display":display,"textures":textures,"queries":queries})
            )?;
        }
    }
    Ok(())
}
