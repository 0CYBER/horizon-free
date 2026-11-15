//! The native mod: the launcher compiles this crate against the pinned game and calls `hooks()`.
//! Each method of `Hooks` is one moment in the game; implement the ones the mod needs
//! (docs/modding/src/start/writing-hooks.md). Need a moment no hook covers? See
//! docs/modding/src/concepts/injections.md.
use mod_api::{self, Hooks, PlayerLoggedInEvent, ServerHost, Text};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    /// A player joined the server (NeoForge's `PlayerLoggedInEvent`): greet them.
    fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
        let line = format!("{id} says hello, {}!", event.name);
        if let Some(player) = host.player(event.player) {
            let _ = player.tell(host, Text::literal(line));
        }
    }
}
