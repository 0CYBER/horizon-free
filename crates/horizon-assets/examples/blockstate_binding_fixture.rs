use horizon_assets::blockstate::{
    BlockStateDefinition,
    bind::{Properties, PropertySchema, PropertyValues, SelectedModels},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let facts: BTreeMap<String, Value> = serde_json::from_reader(File::open(&args[3])?)?;
    let blocks: BTreeMap<_, _> = facts
        .iter()
        .map(|(key, value)| {
            let schema: PropertySchema = value["schema"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(key, value)| {
                    let values = if let Some(range) = value.get("integer") {
                        PropertyValues::Integer {
                            minimum: range[0].as_i64().unwrap() as i32,
                            maximum: range[1].as_i64().unwrap() as i32,
                        }
                    } else {
                        PropertyValues::Named(
                            value["named"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|v| v.as_str().unwrap().to_owned())
                                .collect(),
                        )
                    };
                    (key.clone(), values)
                })
                .collect();
            let states: Vec<Properties> = serde_json::from_value(value["states"].clone()).unwrap();
            (key, (schema, states))
        })
        .collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut output = BufWriter::new(File::create(&args[2])?);
    for line in input.lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let (schema, states) = &blocks[&row["block"].as_str().unwrap().to_owned()];
        let value = BlockStateDefinition::from_json(row["json"].as_str().unwrap())
            .and_then(|v| v.bind(schema, states));
        let result = match value {
            Ok(selected) => {
                json!({"name":row["name"],"selected":selected.iter().map(|s| match s {None=>Value::Null,Some(SelectedModels::Variant(index))=>json!({"variant":index}),Some(SelectedModels::Multipart(indices))=>json!({"multipart":indices})}).collect::<Vec<_>>()})
            }
            Err(_) => json!({"name":row["name"],"error":true}),
        };
        writeln!(output, "{result}")?;
    }
    Ok(())
}
