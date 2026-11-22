//! One integrated server tick, ordered with both halves of the local connection.

use super::{EventLoopCtl, Game, Result};
use crate::game::integrated::{ExportLockstep, Integrated};
use horizon_client::net::EXPORT_FENCE;
use horizon_protocol::packets::play::Serverbound;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

const TICK_TIMEOUT: Duration = Duration::from_secs(5);

// Delayed echoes from a cancelled export must not complete a new export's turn.
fn next_fence() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub(super) struct LiveExport {
    clock: ExportLockstep,
    conn: u64,
    start: Option<(u64, Instant)>,
    steps: u32,
    finish_frame: bool,
    turn: Option<Turn>,
}

struct Turn {
    deadline: Instant,
    fence: u64,
    received: bool,
    tick: Option<u64>,
}

impl LiveExport {
    pub(super) fn new(server: &Integrated, conn: u64) -> Result<Self> {
        let fence = next_fence();
        Ok(Self {
            clock: server.export_lockstep(fence)?,
            conn,
            start: Some((fence, Instant::now() + TICK_TIMEOUT)),
            steps: 0,
            finish_frame: false,
            turn: None,
        })
    }

    pub(super) fn begin_frame(&mut self, steps: u32) {
        self.steps = steps;
        self.finish_frame = true;
    }

    pub(super) fn started(&self) -> bool {
        self.start.is_none()
    }

    fn waiting(&self) -> bool {
        !self.started() || self.turn.is_some()
    }

    pub(super) fn ready_for_frame(&self) -> bool {
        self.started() && self.turn.is_none() && self.steps == 0
    }

    fn check(&self, game: &Game<'_>) -> Result<()> {
        self.clock.check()?;
        if game.options.paced_by_split || game.options.split_guest {
            return Err("splitscreen started during export".into());
        }
        if game.replay.playing()
            || !game
                .remote
                .as_ref()
                .is_some_and(|r| r.session.serial() == self.conn)
        {
            return Err("the local connection changed during export".into());
        }
        self.check_deadline(Instant::now())
    }

    fn check_deadline(&self, now: Instant) -> Result<()> {
        if self.start.is_some_and(|(_, deadline)| now >= deadline)
            || self.turn.as_ref().is_some_and(|turn| now >= turn.deadline)
        {
            return Err("the integrated server or its connection stalled during export".into());
        }
        Ok(())
    }
}

/// The client may retire a fenced batch only once the server turn is complete.
pub(in crate::game) enum ExportIngress {
    Blocked,
    Through(u64),
}

impl LiveExport {
    fn ingress(&self, frame_pending: bool) -> Option<ExportIngress> {
        if let Some((fence, _)) = self.start {
            return Some(ExportIngress::Through(fence));
        }
        let Some(turn) = &self.turn else {
            // The completed turn's world must survive settling and draw retries.
            return frame_pending.then_some(ExportIngress::Blocked);
        };
        Some(
            if turn.received || turn.tick.is_some_and(|tick| !self.clock.completed(tick)) {
                ExportIngress::Blocked
            } else {
                ExportIngress::Through(turn.fence)
            },
        )
    }
}

impl super::Session {
    fn live_ingress(&self) -> Option<ExportIngress> {
        self.live.as_ref()?.ingress(self.pending.is_some())
    }
}

fn backlog_delays_frame(ingress: Option<ExportIngress>, backlog: usize) -> bool {
    // Blocked packets belong to a future turn. The current frame still waits
    // on terrain work already started by packets it applied through its fence.
    backlog != 0 && !matches!(ingress, Some(ExportIngress::Blocked))
}

impl Game<'_> {
    pub(in crate::game) fn export_ingress(&self) -> Option<ExportIngress> {
        self.export.as_ref()?.live_ingress()
    }

    pub(in crate::game) fn remote_backlog_delays_frame(&self) -> bool {
        backlog_delays_frame(self.export_ingress(), self.remote_backlog())
    }

    pub(in crate::game) fn note_export_fence(&mut self, fence: u64) {
        let Some(live) = self.export.as_mut().and_then(|s| s.live.as_mut()) else {
            return;
        };
        if live.start.is_some_and(|(expected, _)| expected == fence) {
            live.start = None;
        } else if let Some(turn) = &mut live.turn
            && turn.fence == fence
        {
            turn.received = true;
        }
    }

    pub(super) fn poll_live_export(&mut self) -> Result<()> {
        if let Some(live) = self.export.as_ref().and_then(|s| s.live.as_ref()) {
            live.check(self)?;
            if self.export_held() {
                self.poll_remote()?;
            }
        }
        Ok(())
    }

    pub(in crate::game) fn take_live_frame_update(&mut self) -> bool {
        self.export
            .as_mut()
            .and_then(|s| s.live.as_mut())
            .is_some_and(|live| std::mem::take(&mut live.finish_frame))
    }

    pub(in crate::game) fn live_export_waiting(&self) -> bool {
        self.export
            .as_ref()
            .and_then(|s| s.live.as_ref())
            .is_some_and(LiveExport::waiting)
    }

    fn queue_export_loading_tick(&mut self) -> Result<()> {
        let Some(session) = &self.export else {
            return Ok(());
        };
        if !session.settle
            || !session
                .live
                .as_ref()
                .is_some_and(|live| live.started() && live.steps == 0)
            || !self
                .remote
                .as_ref()
                .is_some_and(|r| r.level_load.is_active())
        {
            return Ok(());
        }
        // The local readiness delay runs on its own. A credit is needed only for
        // load packets that cannot arrive until the server takes another turn.
        self.export_ready()?;
        let has_column = self.player.as_ref().is_some_and(|p| {
            self.world
                .chunks
                .contains_key(&super::super::player_chunk(p.position))
        });
        if self
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.needs_load_packets(has_column))
        {
            self.export.as_mut().unwrap().live.as_mut().unwrap().steps = 1;
        }
        Ok(())
    }

    pub(super) fn advance_live_export(&mut self, event_loop: &dyn EventLoopCtl) -> Result<bool> {
        self.queue_export_loading_tick()?;
        loop {
            let Some(live) = self.export.as_ref().and_then(|s| s.live.as_ref()) else {
                // Applying a disconnect can interrupt the session inside poll_remote.
                return Ok(false);
            };
            live.check(self)?;
            if live.steps == 0 {
                return Ok(true);
            }
            let live = self.export.as_mut().and_then(|s| s.live.as_mut()).unwrap();
            if live.turn.is_none() {
                // Everything the client sent before this marker must reach server ingress
                // before we grant the tick that is meant to run it.
                let fence = next_fence();
                self.remote
                    .as_ref()
                    .unwrap()
                    .session
                    .send(Serverbound::Unknown(
                        EXPORT_FENCE,
                        fence.to_be_bytes().to_vec(),
                    ));
                live.turn = Some(Turn {
                    deadline: Instant::now() + TICK_TIMEOUT,
                    fence,
                    received: false,
                    tick: None,
                });
            }
            self.poll_remote()?;
            let Some(live) = self.export.as_mut().and_then(|s| s.live.as_mut()) else {
                return Ok(false);
            };
            if !live.advance_turn()? {
                return Ok(false);
            }
            self.client_tick_steps(1, Some(event_loop))?;
        }
    }
}

impl LiveExport {
    fn advance_turn(&mut self) -> Result<bool> {
        self.clock.check()?;
        let turn = self.turn.as_mut().unwrap();
        if !turn.received {
            return Ok(false);
        }
        let Some(tick) = turn.tick else {
            turn.fence = next_fence();
            turn.received = false;
            turn.tick = Some(self.clock.request(turn.fence)?);
            return Ok(false);
        };
        if !self.clock.completed(tick) {
            return Ok(false);
        }
        self.turn = None;
        self.steps -= 1;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Requests, check_export_server_command, parse_start};
    use super::*;
    use crate::game::integrated::with_export_clock;

    #[test]
    fn held_export_startup_fence_refuses_synchronous_saves() {
        with_export_clock(|server| {
            let start = parse_start(&["take", "--size", "16x16", "--fps", "20", "--hold"]).unwrap();
            let requests = Requests {
                hold: start.hold,
                ..Requests::default()
            };
            let mut live = LiveExport::new(server, 1).unwrap();
            assert!(requests.held());
            assert!(live.waiting());
            for command in [["save"], ["save-all"]] {
                assert!(requests.check_command(&command).is_ok());
                assert!(check_export_server_command(live.waiting(), &command).is_err());
            }
            for command in [["export", "status"], ["export", "stop"]] {
                assert!(check_export_server_command(live.waiting(), &command).is_ok());
            }

            live.start = None;
            for command in [["save"], ["save-all"]] {
                assert!(check_export_server_command(live.waiting(), &command).is_ok());
            }
        });
    }

    #[test]
    fn a_credited_tick_keeps_ingress_closed_until_the_server_finishes() {
        with_export_clock(|server| {
            let mut live = LiveExport::new(server, 1).unwrap();
            live.start = None;
            live.turn = Some(Turn {
                deadline: Instant::now() + TICK_TIMEOUT,
                fence: 91,
                received: false,
                tick: Some(live.clock.request(91).unwrap()),
            });
            assert!(matches!(live.ingress(false), Some(ExportIngress::Blocked)));
            assert!(!live.ready_for_frame());
        });
    }

    #[test]
    fn post_fence_packets_wait_across_capture_retries() {
        with_export_clock(|server| {
            let root = tempfile::tempdir().unwrap();
            let mut session = super::super::tests::test_session(root.path());
            let mut live = LiveExport::new(server, 1).unwrap();
            live.start = None;
            session.live = Some(live);
            session.begin_capture();

            // The final turn already completed, but the frame must still settle.
            for _ in 0..2 {
                assert!(matches!(
                    session.live_ingress(),
                    Some(ExportIngress::Blocked)
                ));
                assert!(!backlog_delays_frame(session.live_ingress(), 1));
                assert_eq!(session.pending, Some(0));
            }
            session.capture_submitted();
            assert!(session.live_ingress().is_none());
            assert!(backlog_delays_frame(session.live_ingress(), 1));
        });
    }

    #[test]
    fn stalled_export_turn_releases_the_server_when_the_session_drains() {
        with_export_clock(|server| {
            let root = tempfile::tempdir().unwrap();
            let mut session = super::super::tests::test_session(root.path());
            let deadline = Instant::now() + TICK_TIMEOUT;
            let mut live = LiveExport::new(server, 1).unwrap();
            live.start = None;
            live.turn = Some(Turn {
                deadline,
                fence: 5,
                received: false,
                tick: Some(live.clock.request(5).unwrap()),
            });
            assert!(
                live.check_deadline(deadline - Duration::from_millis(1))
                    .is_ok()
            );
            assert!(
                live.check_deadline(deadline)
                    .unwrap_err()
                    .to_string()
                    .contains("stalled")
            );
            session.live = Some(live);
            assert!(server.export_lockstep(0).is_err());
            session.begin_drain("failed", None);
            assert!(server.export_lockstep(0).is_ok());
        });
    }
}
