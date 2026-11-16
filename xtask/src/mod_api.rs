//! `mod-api`: fill new native hooks' placeholder bodies in the hand-written source, and write
//! the layouts of the foreign types that cross to wasm mods from the locked source.
use std::path::{Path, PathBuf};

fn checkout() -> Result<PathBuf, String> {
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    root.ancestors()
        .find(|path| path.join("crates/horizon-mod-vocab/src/hooks").is_dir())
        .map(Path::to_path_buf)
        .ok_or("run xtask from within the Horizon checkout".into())
}

pub fn run() -> Result<(), String> {
    let root = checkout()?;
    horizon_mod_api_gen::fill_hooks(&root)?;
    horizon_mod_api_gen::crossing::write_foreign_layouts(&root)
}

/// Prints the wasm boundary coverage chapter for the book preprocessor, without writing a source
/// file.
pub fn coverage() -> Result<(), String> {
    horizon_mod_api_gen::generated_on_stdout();
    print!(
        "{}",
        horizon_mod_api_gen::generate_wire_coverage(&checkout()?)?
    );
    Ok(())
}

/// Prints the hook-notes chapter for the book preprocessor, without writing a source file.
pub fn notes() -> Result<(), String> {
    horizon_mod_api_gen::generated_on_stdout();
    print!(
        "{}",
        horizon_mod_api_gen::generate_hook_notes(&checkout()?)?
    );
    Ok(())
}
