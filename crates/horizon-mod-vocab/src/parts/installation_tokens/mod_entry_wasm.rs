//! Cold registration construction, with native representation kept private.
/// Whether the game loaded the mod as a wasm module.
pub fn mod_entry_wasm(entry: &crate::ModEntry) -> bool {
    entry.wasm
}
