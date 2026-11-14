//! The keys whose presses send a packet on a server's client (swap hands, drop, a mount's
//! inventory): queued by the key handling, acted on in the tick's `handleKeybinds` phase.
use super::{Game, S};
use horizon_client::input_queue::QueueLimits;
use horizon_protocol::packets::play;
use winit::keyboard::KeyCode;

/// The bounds of `Remote::pending_keys`: a tick acts on 64 presses, oldest first, and 1024 wait at
/// most. A player presses a few keys a tick.
pub(super) const PENDING_KEYS: QueueLimits = QueueLimits {
    name: "remote key presses",
    max_len: 1024,
    per_tick: 64,
};

impl Game<'_> {
    pub(super) fn remote_queued_keys(&mut self) {
        // handleKeybinds' keybind polls: the queued presses act now, in tick
        // order, so no packet goes out between CLIENT_TICK_END and a pong.
        let pending = self
            .remote
            .as_mut()
            .map(|r| r.pending_keys.take_tick())
            .unwrap_or_default();
        for key in pending {
            match key {
                KeyCode::KeyF if !self.spectator() => self.remote_swap_hands(),
                KeyCode::KeyQ if !self.spectator() => {
                    // Minecraft.hasControlDown: the physical keys, so a toggled
                    // sprint on Left Control does not make Q drop the stack.
                    let all = self.has_control_down() || self.input_takes_stack_drop();
                    self.remote_drop(all);
                }
                KeyCode::KeyE
                    if !self.in_portal_transition()
                        && (self.riding_mount_with_menu() || self.riding_chest_boat()) =>
                {
                    let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
                    self.send(S::PlayerCommand(play::PlayerCommand {
                        id,
                        action: 5,
                        data: 0,
                    }));
                }
                _ => {}
            }
        }
        if self.clicks.miss_time > 0 {
            self.clicks.miss_time -= 1;
        }
    }
}
