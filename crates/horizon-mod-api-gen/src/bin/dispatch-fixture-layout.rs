//! Lossless fixture ranges for `tools/split_dispatch_fixtures.py`.
fn main() -> Result<(), String> {
    use std::io::Read;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut source = String::new();
    std::io::stdin()
        .read_to_string(&mut source)
        .map_err(|e| e.to_string())?;
    println!(
        "{}",
        horizon_mod_api_gen::dispatch_fixture_layout(&root, &source)?
    );
    Ok(())
}
