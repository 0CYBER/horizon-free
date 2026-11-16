//! The native mod the "Your first native mod" page builds: `cargo horizon-mod new greeter --template native`.
// ANCHOR: mod
use mod_api::{self, Hooks, PlayerLoggedInEvent, ServerHost, Text};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    /// A player joined the server: greet them.
    fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
        let line = format!("greeter says hello, {}!", event.name);
        if let Some(player) = host.player(event.player) {
            let _ = player.tell(host, Text::literal(line));
        }
    }
}
// ANCHOR_END: mod
