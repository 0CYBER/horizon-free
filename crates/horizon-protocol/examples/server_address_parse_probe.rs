//! Host/port parser corpus, without DNS or a connection.
use horizon_protocol::servers::{is_valid_address, parse_address};
use std::{error::Error, fs::File};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = args.get(1).ok_or("expected input and output paths")?;
    let output = args.get(2).ok_or("expected output path")?;
    let addresses: Vec<String> = serde_json::from_reader(File::open(input)?)?;
    let results: Vec<_> = addresses
        .iter()
        .map(|text| {
            serde_json::json!({
                "valid": is_valid_address(text),
                "parsed": parse_address(text),
            })
        })
        .collect();
    serde_json::to_writer(File::create(output)?, &results)?;
    Ok(())
}
