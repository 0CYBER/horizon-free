//! The mod's own crate. Its `[[native.mixin]]` helpers (`src/mixins/`) are compiled into copies of the
//! game functions they patch, not into this crate.
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}

/// Called by a helper: a helper may use its mod's crate.
pub fn note(what: &str) {
    eprintln!("fnr: loop_generic_head {what}");
}
