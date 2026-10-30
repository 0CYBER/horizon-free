#![forbid(unsafe_code)]
//! Runs tools/test_fire_starter_sound_parity.py's level random seeds through
//! the fire charge's use sound (`horizon_client::items::fire_starter_pitch`).
use horizon_client::{
    Result,
    items::{fire_starter_pitch, fire_starter_sound},
};
use horizon_core::random::{LegacyRandom, RandomSource};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut out = BufWriter::new(File::create(&args[2])?);
    for line in input.lines() {
        let mut random = LegacyRandom::new(line?.trim().parse()?);
        let pitch = fire_starter_pitch(true, &mut random);
        writeln!(
            out,
            "{} {} {} {}",
            fire_starter_sound(true),
            1.0f32.to_bits() as i32,
            pitch.to_bits() as i32,
            random.next_long()
        )?;
    }
    out.flush()?;
    Ok(())
}
