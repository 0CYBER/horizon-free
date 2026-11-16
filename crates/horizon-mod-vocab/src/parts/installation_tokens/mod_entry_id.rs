//! Cold registration construction, with native representation kept private.
/// Construct or inspect the owned registration value.
pub fn mod_entry_id(entry: &crate::ModEntry) -> &'static str {
    entry.id
}
