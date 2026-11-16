//! Generate API sources and WIT without compiling the game: api-layout ROOT OUTPUT.
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("usage: api-layout ROOT OUTPUT")?);
    let output = PathBuf::from(args.next().ok_or("usage: api-layout ROOT OUTPUT")?);
    if args.next().is_some() {
        return Err("usage: api-layout ROOT OUTPUT".into());
    }
    std::fs::create_dir_all(&output)?;
    horizon_mod_api_gen::generate(&root, &output, horizon_mod_api_gen::Target::Api)?;
    Ok(())
}
