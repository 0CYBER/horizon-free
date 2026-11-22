//! The Rust the modding book includes with `{{#include}}`. A snippet lives here, between
//! `ANCHOR` comments, so a change to the mod API that breaks it fails the build instead of
//! leaving a stale page.

use mod_api::{AttachmentError, AttachmentType, Holder, Hooks, ServerHost, ServerTickEvent};

#[derive(Default)]
struct Mod;

// ANCHOR: counter
/// A counter on the level, saved with the world.
static COUNTER: AttachmentType<i32> = AttachmentType {
    namespace: "example",
    name: "counter",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};
// ANCHOR_END: counter

// ANCHOR: tick_hook
#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn server_tick(&self, host: &mut dyn ServerHost, _: &mut ServerTickEvent<'_>) {
        let _ = count(host);
    }
}
// ANCHOR_END: tick_hook

// ANCHOR: count
fn count(host: &mut dyn ServerHost) -> Result<(), AttachmentError> {
    let old = COUNTER.get(&mut *host, Holder::Level)?;
    COUNTER.set(host, Holder::Level, old + 1)
}
// ANCHOR_END: count

// ANCHOR: menu_quick_move
/// Shift-click a slot in the local player's current menu.
pub fn quick_move(host: &mut dyn mod_api::ClientHost, slot: usize) -> bool {
    let Some(menu) = host
        .local_player()
        .and_then(|player| player.container_menu(host))
    else {
        return false;
    };
    menu.click(host, &[mod_api::MenuClick::QuickMove { slot }])
}
// ANCHOR_END: menu_quick_move

// Every standalone snippet module under src is compiled.
include!(concat!(env!("OUT_DIR"), "/example_modules.rs"));
