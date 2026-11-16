//! A `[[native.loop]]` slot (`src/loops/tenth.rs`) that panics on the tenth hopper it sees: the
//! server crashes, and the crash report names this mod and the loop.
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}
