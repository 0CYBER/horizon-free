mod support;
use horizon_assets::model::CuboidModel;
use serde_json::json;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};
use support::model;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut output = BufWriter::new(File::create(&args[2])?);
    for line in input.lines() {
        let row: Vec<String> = serde_json::from_str(&line?)?;
        let result = match CuboidModel::from_json(&row[1]) {
            Ok(value) => json!({"name":row[0],"model":model(&value)}),
            Err(_) => json!({"name":row[0],"error":true}),
        };
        writeln!(output, "{result}")?;
    }
    Ok(())
}
