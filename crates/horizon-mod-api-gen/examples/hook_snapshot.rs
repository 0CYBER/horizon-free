//! Capture generated hook artifacts for a byte-for-byte refactor comparison.
//! Run with a destination directory argument before and after the split, then `diff -r`.
use std::{fs, path::PathBuf};
fn main() -> Result<(), String> {
    let root = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let out = PathBuf::from(std::env::args().nth(1).expect("destination directory"));
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    fs::write(
        out.join("points.json"),
        serde_json::to_vec(&horizon_mod_api_gen::points::game_points(&root)?)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    horizon_mod_api_gen::generate(&root, &out, horizon_mod_api_gen::Target::Api)?;
    horizon_mod_api_gen::generate_points(&root, &out)?;
    fs::write(
        out.join("catalog.json"),
        horizon_mod_api_gen::neoforge_catalog(&root)?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        out.join("hook-notes.md"),
        horizon_mod_api_gen::generate_hook_notes(&root)?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
