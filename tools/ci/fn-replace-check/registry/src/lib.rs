//! A mod with a crates.io dependency (`itoa`) whose `[[native.mixin]]` helper calls into it.
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}

/// Called by a helper; formats with the mod's own registry dependency.
pub fn note(what: &str, value: i64) {
    eprintln!("fnr: registry {what} {}", itoa::Buffer::new().format(value));
}
