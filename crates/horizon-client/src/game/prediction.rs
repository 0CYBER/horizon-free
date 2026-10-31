//! The movement prediction's replay on the server (`lan::prediction`): each guest's client ticks
//! played again with the client's own movement code, in the guest's level.
use super::Game;
use super::lan::LanHost;
use horizon_client::lan::prediction::Replay;
use horizon_client::motion::MotionWorld;

impl Game<'_> {
    /// Guest `conn`'s client ticks since the server's last tick, played again, before the server
    /// moves the guest's own body.
    pub(super) fn guest_replay(&mut self, host: &mut LanHost, conn: u32) {
        if !host.hub.recording() {
            return;
        }
        let Some(g) = host.hub.guest_mut(conn) else {
            return;
        };
        // What moves the player isn't its own movement (a vehicle, a bed), or the server holds it:
        // the replay starts again at the teleport that ends it (dismountTo, stopSleeping, respawn).
        let held = g.vehicle.is_some()
            || g.sleeping
            || g.dead
            || g.spectator()
            || g.awaiting_teleport.is_some()
            || !g.client_loaded();
        let (Some(record), Some(server)) = (g.anticheat.record.as_deref_mut(), g.motion.as_deref())
        else {
            return;
        };
        if let Some(at) = record.take_restart() {
            g.replay = Some(Box::new(Replay::new(server, at, record.ended())));
        }
        if held {
            g.replay = None;
            return;
        }
        let Some(replay) = g.replay.as_deref_mut() else {
            return;
        };
        // Ticks no longer kept (a server that stalled for seconds): wait for the next teleport.
        let Some(ticks) = record.since(replay.played()) else {
            g.replay = None;
            return;
        };
        for tick in ticks {
            let run = |body: &mut _, keys, sprinting| {
                MotionWorld {
                    entities: &[],
                    player: body,
                    scene: &mut self.world,
                    assets: &self.pack.assets,
                    collision_binding: self.collision,
                    resources: self.motion,
                }
                .replayed_tick(keys, sprinting)
            };
            if replay.play(tick, server, run).is_none() {
                g.replay = None;
                return;
            }
        }
    }
}
