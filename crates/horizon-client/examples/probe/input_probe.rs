#![forbid(unsafe_code)]
use horizon_client::{Result, input};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};
fn number(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn bits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc00000 } else { v.to_bits() }
}
pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().skip(1).collect();
    let mut out = BufWriter::new(File::create(&args[1])?);
    for line in BufReader::new(File::open(&args[0])?).lines() {
        let f: Value = serde_json::from_str(&line?)?;
        let keys = f["keys"].as_u64().unwrap();
        let direction =
            input::keyboard_direction(keys & 1 != 0, keys & 2 != 0, keys & 4 != 0, keys & 8 != 0);
        let vector = if f["vector"].is_null() {
            direction
        } else {
            [number(&f["vector"][0]), number(&f["vector"][1])]
        };
        let result = input::modify_input(
            vector,
            (f["using"].as_bool().unwrap() && !f["riding"].as_bool().unwrap())
                .then(|| number(&f["item"])),
            f["slow"].as_bool().unwrap().then(|| number(&f["sneak"])),
        );
        writeln!(
            out,
            "{:08x} {:08x} {:08x} {:08x}",
            bits(direction[0]),
            bits(direction[1]),
            bits(result[0]),
            bits(result[1])
        )?;
    }
    Ok(())
}
