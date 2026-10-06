//! Text transport for native BiomeManager comparisons.
use horizon_core::{
    biome::{BiomeZoom, noise_quart_at_block, noise_quart_at_position, obfuscate_seed},
    position::BlockPos,
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let mut values = line.split_whitespace();
        match values.next().unwrap() {
            "s" => writeln!(out, "s {}", obfuscate_seed(values.next().unwrap().parse()?))?,
            "b" => {
                let seed: i64 = values.next().unwrap().parse()?;
                let xyz: Vec<i32> = values.map(str::parse).collect::<Result<_, _>>()?;
                let pos = BlockPos::new(xyz[0], xyz[1], xyz[2]);
                let zoom = BiomeZoom::new(seed);
                write!(out, "b")?;
                for value in zoom
                    .quart_at_block(pos)
                    .into_iter()
                    .chain(noise_quart_at_block(pos))
                    .chain(zoom.quart_at_block(pos))
                {
                    write!(out, " {value}")?;
                }
                writeln!(out)?;
            }
            "p" => {
                let bits: Vec<_> = values
                    .map(|v| u64::from_str_radix(v, 16).map(f64::from_bits))
                    .collect::<Result<_, _>>()?;
                let xyz = noise_quart_at_position(bits.try_into().unwrap());
                writeln!(out, "p {} {} {}", xyz[0], xyz[1], xyz[2])?;
            }
            _ => panic!("unknown fixture operation"),
        }
    }
    Ok(())
}
