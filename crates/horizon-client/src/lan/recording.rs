//! The movement record's upkeep on the server's hub (docs/anticheat.md): while the anticheat's
//! prediction runs, each guest's client ticks (its moves, and its pongs to the ping that ends each server
//! tick) go into its record (`lan::prediction::record`), which the engine's movement replay plays again.
use super::hub::Hub;
use horizon_anticheat::Check;

impl Hub {
    /// Whether the prediction records the players' client ticks.
    pub fn recording(&self) -> bool {
        self.anticheat
            .as_deref()
            .is_some_and(|ac| ac.config.runs(Check::Prediction))
    }

    /// A move packet from guest `i`, as it came: the prediction's record of the client tick.
    pub(super) fn anticheat_moved(
        &mut self,
        i: usize,
        pos: Option<[f64; 3]>,
        rot: Option<(f32, f32)>,
        flags: u8,
    ) {
        if self.recording() {
            self.guests.update(i, |g| {
                g.record.get_or_insert_default().moved(pos, rot, flags);
            });
        }
    }

    /// ServerboundPongPacket: the client has handled every packet of the server tick `id` (the ping
    /// [`Hub::anticheat_ping`] sent at its end).
    pub(super) fn anticheat_pong(&mut self, i: usize, id: i32) {
        let ticks = self.ticks;
        if self.recording() {
            self.guests.update(i, |g| {
                // Ids are server ticks, cut to 32 bits.
                g.record
                    .get_or_insert_default()
                    .pong(u64::from(id as u32), ticks);
            });
        }
    }

    /// The end of a server tick, after everything it sent: a ping whose id is the tick to each
    /// player in the world, so the prediction knows from which client tick on that tick's packets
    /// were in effect.
    pub fn anticheat_ping(&mut self) {
        if !self.recording() {
            return;
        }
        // What the tick sent the players that changes their movement went out before this ping.
        let ticks = self.ticks;
        self.guests.update_all(|g| {
            let changes = std::mem::take(
                g.changes
                    .get_mut()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            // The ping below goes to the players in the world.
            let placed = g.placed;
            if let Some(record) = g.record.as_deref_mut() {
                for change in changes {
                    record.changed(ticks, change);
                }
                if placed {
                    record.pinged(ticks);
                }
            }
        });
        let ping = horizon_protocol::packets::play::Clientbound::Ping(
            horizon_protocol::packets::configuration::Ping {
                id: self.ticks as u32 as i32,
            },
        );
        for g in self.guests.iter().filter(|g| g.placed) {
            g.send(&ping);
        }
    }
}
