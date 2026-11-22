//! Bitmap glyph advances for `tools/test_bitmap_advance_parity.py`.
use horizon_assets::resource_source::ResourceSource;
use horizon_client::font::Font;
use std::{error::Error, fs, io::Write, path::PathBuf};

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = crate::args().collect();
    let root = PathBuf::from(&args[1]);
    let providers: serde_json::Value = serde_json::from_str(&fs::read_to_string(
        root.join("assets/minecraft/font/default.json"),
    )?)?;
    let font = Font::metrics(&ResourceSource::directory(root));
    let mut out = fs::File::create(&args[2])?;
    for index in 0..providers["providers"].as_array().map_or(0, Vec::len) {
        let glyph = font.glyph(char::from_u32(0xE000 + index as u32).unwrap());
        writeln!(out, "{:.3}", glyph.advance)?;
    }
    Ok(())
}
