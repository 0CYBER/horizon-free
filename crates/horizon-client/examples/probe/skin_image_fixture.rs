//! Headless transport for the skin half of the player list parity gate:
//! decodes each base64 PNG and runs `horizon_client::skins::process_legacy_skin`,
//! writing the RGBA result in base64, or "refused".
#![forbid(unsafe_code)]
use std::{
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn main() -> Result<()> {
    let args: Vec<PathBuf> = crate::args_os().skip(1).map(PathBuf::from).collect();
    let [inputs, output] = args.as_slice() else {
        return Err("usage: skin_image_fixture INPUTS OUTPUT".into());
    };
    let mut out = BufWriter::new(fs::File::create(output)?);
    for line in fs::read_to_string(inputs)?
        .lines()
        .filter(|l| !l.is_empty())
    {
        let png = horizon_client::skins::base64_decode(line).ok_or("bad base64")?;
        let result = horizon_client::hud::decode_bytes(&png)
            .ok()
            .and_then(|(w, h, rgba)| horizon_client::skins::process_legacy_skin(w, h, &rgba));
        match result {
            Some(rgba) => writeln!(out, "{}", horizon_client::lan::base64(&rgba))?,
            None => writeln!(out, "refused")?,
        }
    }
    Ok(())
}
