//! The integrated server's tick credits while a live export owns its clock.

use super::{Flags, Game, Integrated, Task};
use horizon_client::{Result, net::EXPORT_FENCE};
use horizon_protocol::buf::Writer;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::Sender,
    },
    time::Instant,
};

#[derive(Default)]
pub(super) struct Slot(Mutex<Option<Arc<Ticks>>>, AtomicBool);

struct RunningExportTick<'a>(&'a AtomicBool);

impl Drop for RunningExportTick<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Slot {
    pub(super) fn active(&self) -> Option<Arc<Ticks>> {
        self.0.lock().unwrap().clone()
    }

    fn begin_tick(&self, ticks: &Ticks) -> Option<RunningExportTick<'_>> {
        // Claim under the same lock as lease release. A cancelled credit
        // cannot start after the client sees that the server is idle.
        let active = self.0.lock().unwrap();
        if !active
            .as_deref()
            .is_some_and(|current| std::ptr::eq(current, ticks))
        {
            return None;
        }
        self.1.store(true, Ordering::Release);
        Some(RunningExportTick(&self.1))
    }

    pub(super) fn interrupt(&self) {
        if let Some(ticks) = self.0.lock().unwrap().take() {
            ticks.release();
        }
    }
}

pub(super) struct Ticks {
    tasks: Sender<Task>,
    interrupted: AtomicBool,
    start_fence: AtomicU64,
    // At most one credit is outstanding. Its value is the outgoing packet fence.
    credit: AtomicU64,
    done: AtomicU64,
}

impl Ticks {
    fn wake(&self) -> bool {
        // The task queue also wakes the server for hold changes and other requests.
        self.tasks.send(Box::new(|_| {})).is_ok()
    }

    fn take_credit(&self, held: bool) -> u64 {
        if held {
            0
        } else {
            self.credit.swap(0, Ordering::AcqRel)
        }
    }

    fn release(&self) {
        self.interrupted.store(true, Ordering::Release);
        self.wake();
    }
}

/// Owned by one export session. Releasing a session cannot release a later export.
pub(in crate::game) struct ExportLockstep {
    flags: Arc<Flags>,
    ticks: Arc<Ticks>,
}

impl Integrated {
    pub(in crate::game) fn running_export_tick(&self) -> bool {
        self.flags.export_ticks.1.load(Ordering::Acquire)
    }

    pub(in crate::game) fn export_lockstep(&self, start_fence: u64) -> Result<ExportLockstep> {
        let mut current = self.flags.export_ticks.0.lock().unwrap();
        if self.flags.stop.load(Ordering::Acquire) || self.stopped() {
            return Err("the integrated server stopped".into());
        }
        if current.is_some()
            || self.running_export_tick()
            || self.flags.export_clock.load(Ordering::Acquire)
            || self.published().is_some()
            || self.has_split_players()
        {
            return Err("the integrated server is unavailable for export".into());
        }
        let ticks = Arc::new(Ticks {
            tasks: self.tasks.clone(),
            interrupted: AtomicBool::new(false),
            start_fence: AtomicU64::new(start_fence),
            credit: AtomicU64::new(0),
            done: AtomicU64::new(0),
        });
        *current = Some(ticks.clone());
        Ok(ExportLockstep {
            flags: self.flags.clone(),
            ticks,
        })
    }
}

impl ExportLockstep {
    pub(in crate::game) fn check(&self) -> Result<()> {
        if self.flags.stop.load(Ordering::Acquire) || self.flags.finished.load(Ordering::Acquire) {
            return Err("the integrated server stopped during export".into());
        }
        if self.ticks.interrupted.load(Ordering::Acquire) {
            return Err(
                "the integrated server was opened to LAN or splitscreen during export".into(),
            );
        }
        Ok(())
    }

    pub(in crate::game) fn request(&self, fence: u64) -> Result<u64> {
        self.check()?;
        let target = self.ticks.done.load(Ordering::Acquire) + 1;
        self.ticks.credit.store(fence, Ordering::Release);
        if !self.ticks.wake() {
            return Err("the integrated server stopped during export".into());
        }
        Ok(target)
    }

    pub(in crate::game) fn completed(&self, target: u64) -> bool {
        self.ticks.done.load(Ordering::Acquire) >= target
    }
}

impl Drop for ExportLockstep {
    fn drop(&mut self) {
        let mut current = self.flags.export_ticks.0.lock().unwrap();
        if current
            .as_ref()
            .is_some_and(|ticks| Arc::ptr_eq(ticks, &self.ticks))
        {
            self.ticks.release();
            *current = None;
        }
    }
}

impl Flags {
    pub(super) fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.export_ticks.interrupt();
    }
}

impl Game<'_> {
    /// Runs in the server loop. While held, keep the credit for after the
    /// release and continue serving tasks, including the release request.
    pub(super) fn run_export_turn(&mut self, flags: &Flags, ticks: &Ticks) -> bool {
        if ticks.interrupted.load(Ordering::Acquire) || flags.stop.load(Ordering::Acquire) {
            return true;
        }
        let start = ticks.start_fence.swap(0, Ordering::AcqRel);
        if start != 0 {
            // Any last wall-clock tick and its packets precede the first capture.
            self.send_export_fence(start);
        }
        let credit = ticks.take_credit(self.agent.hold_server);
        if credit == 0 {
            let Some(task) = self.serving.as_ref().and_then(|s| s.tasks.recv().ok()) else {
                return false;
            };
            return self.run_task(flags, task);
        }
        if ticks.interrupted.load(Ordering::Acquire) || flags.stop.load(Ordering::Acquire) {
            return true;
        }
        let Some(_running) = flags.export_ticks.begin_tick(ticks) else {
            return true;
        };
        if flags.run_tick(|| self.tick_for_export(flags)).is_err() {
            return false;
        }
        self.apply_publish();
        self.publish_load_view();
        self.send_export_fence(credit);
        ticks.done.fetch_add(1, Ordering::Release);
        true
    }

    pub(super) fn tick_for_export(&mut self, flags: &Flags) -> Result<()> {
        self.server_tick_deadline = Some(Instant::now() + self.clock.tick_duration());
        self.follow_distances(flags);
        self.tick_server_measured(false)
    }

    fn send_export_fence(&self, fence: u64) {
        if let Some(owner) = self.serving.as_ref().map(|s| s.owner)
            && let Some(guest) = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.uuid == owner))
        {
            // The sender preserves order with the packets flushed by this tick.
            let mut body = Writer::new();
            body.varint(EXPORT_FENCE);
            body.bytes(&fence.to_be_bytes());
            guest.sender.send(body.data);
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::{sync::mpsc, thread, time::Duration};

    pub(in crate::game::integrated) fn with_clock(
        test: impl FnOnce(&Integrated, &mpsc::Receiver<Task>),
    ) {
        let (tasks, receiver) = mpsc::channel();
        let (stop, wait) = mpsc::channel::<()>();
        let server = Integrated {
            flags: Arc::default(),
            thread: Some(thread::spawn(move || {
                let _ = wait.recv();
            })),
            emergency_save: None,
            tasks,
            world_name: String::new(),
            save_folder: String::new(),
            rule_registry: Arc::new(horizon_client::gamerules::RuleRegistry::vanilla()),
        };
        // Release the fixture thread before Integrated joins, including on a failed assertion.
        let _stop = stop;
        test(&server, &receiver);
    }

    #[test]
    fn running_export_tick_stays_busy_after_its_lease_is_released() {
        with_clock(|server, _| {
            let clock = server.export_lockstep(0).unwrap();
            let slot = &server.flags.export_ticks;
            let ticks = slot.active().unwrap();
            let running = slot.begin_tick(&ticks).unwrap();
            drop(clock);

            assert!(server.running_export_tick());
            assert!(server.export_lockstep(0).is_err());
            drop(running);
            assert!(!server.running_export_tick());
            assert!(
                slot.begin_tick(&ticks).is_none(),
                "released work cannot start"
            );
            assert!(server.export_lockstep(0).is_ok());
        });
    }

    #[test]
    fn export_credit_is_consumed_once_after_server_hold_is_released() {
        with_clock(|server, tasks| {
            let clock = server.export_lockstep(0).unwrap();
            let ticks = server.flags.export_ticks.active().unwrap();
            assert_eq!(ticks.take_credit(false), 0);
            assert!(tasks.try_recv().is_err());

            for fence in 41..44 {
                let target = clock.request(fence).unwrap();
                drop(tasks.recv_timeout(Duration::from_secs(1)).unwrap());
                assert_eq!(ticks.take_credit(true), 0);
                assert!(!clock.completed(target));
                assert_eq!(ticks.take_credit(false), fence);
                ticks.done.fetch_add(1, Ordering::Release);
                assert!(clock.completed(target));
                assert_eq!(ticks.take_credit(false), 0);
            }

            drop(clock);
            drop(tasks.recv_timeout(Duration::from_secs(1)).unwrap());
            assert!(server.flags.export_ticks.active().is_none());
        });
    }

    #[test]
    fn export_clock_refuses_shared_worlds_and_wakes_on_shutdown() {
        with_clock(|server, tasks| {
            let clock = server.export_lockstep(0).unwrap();
            server.flags.published.store(25565, Ordering::Relaxed);
            server.flags.export_ticks.interrupt();
            drop(tasks.recv_timeout(Duration::from_secs(1)).unwrap());
            assert!(clock.check().unwrap_err().to_string().contains("LAN"));
            assert!(server.export_lockstep(0).is_err());

            server.flags.published.store(0, Ordering::Relaxed);
            server.flags.split_players.lock().unwrap().push(1);
            assert!(server.export_lockstep(0).is_err());
            server.flags.split_players.lock().unwrap().clear();
            let next = server.export_lockstep(0).unwrap();
            drop(clock);
            assert!(next.check().is_ok(), "an old lease released a new export");
            let target = next.request(52).unwrap();
            drop(tasks.recv_timeout(Duration::from_secs(1)).unwrap());
            server.request_stop();
            drop(tasks.recv_timeout(Duration::from_secs(1)).unwrap());
            assert!(next.check().unwrap_err().to_string().contains("stopped"));
            assert!(!next.completed(target));
            assert!(server.flags.export_ticks.active().is_none());
        });
    }
}
