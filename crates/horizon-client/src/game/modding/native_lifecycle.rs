//! World epochs for native handles, and lifecycle observations on authoritative servers.
//! A paused pass
//! observes nothing; a frozen world reports `world_running: false` except during a step.

use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    Game,
    execution_role::ExecutionRole,
    scheduled_tasks::{ScheduledTasks, run_all},
};
use mod_api::{ScheduleError, TaskId};
// Their old home, for mixin code that names them here.
pub(super) use mod_api::{
    Authority, LifecycleContext, ServerTickPhase, TickId, WorldEpoch, WorldPhase,
};

/// Epoch allocation spans server and client `Game` instances and client level
/// replacements, so the counter is process-wide.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// A process-wide epoch no handle has carried yet: a server open's, or a
/// client level's (`native_entity_refs.rs`), so a handle never crosses sides.
pub(super) fn fresh_epoch() -> WorldEpoch {
    WorldEpoch::new(NEXT_EPOCH.fetch_add(1, Ordering::Relaxed))
}

/// Internal state for deciding which events can fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenState {
    Opening,
    Ready,
    Closing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OpenAttempt {
    authority: Authority,
    epoch: u64,
    session: OpenState,
    ticks: u64,
    /// A tick started and its end has not fired (or the tick errored).
    tick_open: bool,
    tick_world_running: bool,
}

impl OpenAttempt {
    fn context(&self) -> LifecycleContext {
        LifecycleContext::from(mod_api::__private::built::LifecycleContext {
            authority: self.authority,
            epoch: WorldEpoch::new(self.epoch),
            tick: TickId::new(self.ticks),
        })
    }

    fn server_context(&self) -> Option<LifecycleContext> {
        (self.authority != Authority::RemoteClient).then(|| self.context())
    }
}

/// The lifecycle bookkeeping: the current attempt's epoch, session phase
/// and non-paused tick count. It only decides whether a phase fires; every
/// hook body is empty, so with no native mods nothing changes.
#[derive(Default)]
pub(super) struct NativeLifecycle {
    attempt: Option<OpenAttempt>,
    read_scope: bool,
    /// The mods' delayed tasks (`schedule_task`): dropped whenever a world opens or closes.
    tasks: ScheduledTasks,
    /// The section `entity_entering_section` files each entity under, for this epoch's world.
    pub(super) entity_sections: super::entity_sections::EntitySections,
    /// Tossed item entities `item_toss` is still asking about, innermost last: a join flush
    /// stops before them, so a cancel never takes back an entity already announced.
    pub(super) held_joins: Vec<u32>,
    /// Item notices waiting for their join decision.
    pub(super) pending_item_spawns: Vec<(u32, String, i32, [f64; 3])>,
    /// The log has named the entity id exhaustion (`entity_ids_exhausted`).
    pub(super) ids_exhausted_told: bool,
}

impl NativeLifecycle {
    /// A read is valid only during a tick hook with its current tick context.
    pub(super) fn permits_read(&self, context: &LifecycleContext) -> bool {
        self.read_scope
            && self
                .attempt
                .as_ref()
                .is_some_and(|a| a.session == OpenState::Ready && a.context() == *context)
    }

    /// The epoch of the world open now, from the start of its open to the end
    /// of its close: what a mod's entity and level handles carry.
    pub(super) fn epoch(&self) -> Option<WorldEpoch> {
        self.attempt.map(|a| WorldEpoch::new(a.epoch))
    }

    /// The open starts: a fresh process-wide epoch, never reused.
    pub(super) fn begin_open(&mut self, authority: Authority) {
        let epoch = fresh_epoch().get();
        self.tasks.clear();
        self.attempt = Some(OpenAttempt {
            authority,
            epoch,
            session: OpenState::Opening,
            ticks: 0,
            tick_open: false,
            tick_world_running: false,
        });
    }

    pub(super) fn world_open_ready(&mut self) -> Option<LifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Opening {
            return None;
        }
        attempt.session = OpenState::Ready;
        attempt.server_context()
    }

    /// `WorldOpenFailed`: only while opening; the attempt is invalidated.
    pub(super) fn world_open_failed(&mut self) -> Option<LifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Opening {
            return None;
        }
        let context = attempt.server_context();
        self.attempt = None;
        self.tasks.clear();
        context
    }

    pub(super) fn begin_close(&mut self) {
        if let Some(attempt) = self.attempt.as_mut() {
            attempt.session = OpenState::Closing;
        }
    }

    /// A failure after readiness retires the epoch, including during shutdown.
    pub(super) fn world_close_failed(&mut self) -> Option<LifecycleContext> {
        let attempt = self.attempt.as_ref()?;
        if attempt.session == OpenState::Opening {
            return None;
        }
        let context = attempt.server_context();
        self.attempt = None;
        self.tasks.clear();
        context
    }

    /// `WorldCloseFinal`: at most once per epoch; the epoch retires.
    pub(super) fn world_close_final(&mut self) -> Option<LifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Closing {
            return None;
        }
        let context = attempt.server_context();
        self.attempt = None;
        self.tasks.clear();
        context
    }

    /// Non-paused ready ticks only; a paused pass observes nothing, does not count.
    pub(super) fn server_tick_start(
        &mut self,
        world_running: bool,
    ) -> Option<(LifecycleContext, ServerTickPhase)> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Ready || attempt.authority == Authority::RemoteClient {
            return None;
        }
        attempt.ticks += 1;
        // "Now" for the mods' delayed tasks moves before any hook of this tick runs.
        self.tasks.begin_tick();
        attempt.tick_open = true;
        attempt.tick_world_running = world_running;
        Some((
            attempt.context(),
            ServerTickPhase::ServerTickStart { world_running },
        ))
    }

    /// Discard the pending end when a tick fails.
    pub(super) fn cancel_server_tick(&mut self) {
        if let Some(attempt) = &mut self.attempt {
            attempt.tick_open = false;
        }
    }

    /// Pairs the last start; an errored tick never reaches it.
    pub(super) fn server_tick_end(
        &mut self,
        paused: bool,
    ) -> Option<(LifecycleContext, ServerTickPhase)> {
        let attempt = self.attempt.as_mut()?;
        if paused || attempt.session != OpenState::Ready || !attempt.tick_open {
            return None;
        }
        attempt.tick_open = false;
        Some((
            attempt.context(),
            ServerTickPhase::ServerTickEnd {
                world_running: attempt.tick_world_running,
            },
        ))
    }
}

impl Game<'_> {
    /// The role is fixed at construction; a mirror `Game` is remote whatever it is.
    fn native_lifecycle_authority(&self) -> Authority {
        if self.remote.is_some() {
            return Authority::RemoteClient;
        }
        match self.options.execution_role {
            ExecutionRole::Client => Authority::LocalAuthoritative,
            ExecutionRole::IntegratedServer => Authority::IntegratedServer,
            ExecutionRole::DedicatedServer => Authority::DedicatedServer,
        }
    }

    pub(super) fn native_lifecycle_open_started(&mut self) {
        self.clear_composter_randoms();
        self.reset_entity_residency();
        self.entity_initialization = Default::default();
        if let Some(remote) = &mut self.remote {
            remote.attachments.clear();
        }
        self.attachment_indexes.clear();
        self.passages.retire_world();
        self.native_lifecycle
            .begin_open(self.native_lifecycle_authority());
    }

    pub(super) fn native_lifecycle_world_ready(&mut self) {
        if let Some(context) = self.native_lifecycle.world_open_ready() {
            self.native_server_started(context.authority == Authority::DedicatedServer);
        }
    }

    /// `error_text` is the engine's own error text, unchanged.
    pub(super) fn native_lifecycle_world_failed(&mut self, error_text: &str) {
        // The server levels that opened before the failure close first.
        if self.remote.is_none() {
            self.native_levels_closing();
        }
        if let Some(context) = self.native_lifecycle.world_open_failed() {
            self.native_world_lifecycle(&context, WorldPhase::WorldOpenFailed, error_text);
        }
        self.attachment_indexes.clear();
        self.passages.retire_world();
        self.resident_entities.table.get_mut().reset();
    }

    /// Report a server-loop or shutdown failure once and retire its epoch.
    pub(super) fn native_lifecycle_close_failed(&mut self, error_text: &str) {
        if self.remote.is_none() {
            self.native_levels_closing();
        }
        if let Some(context) = self.native_lifecycle.world_close_failed() {
            self.native_world_lifecycle(&context, WorldPhase::WorldCloseFailed, error_text);
        }
        self.attachment_indexes.clear();
        self.passages.retire_world();
        self.resident_entities.table.get_mut().reset();
    }

    /// After the final save, before the lock release. Client epochs retire
    /// without dispatching server lifecycle hooks.
    pub(super) fn native_lifecycle_world_closed(&mut self) {
        self.instance_groups.borrow_mut().clear();
        // Each server level closes after the final save (`LevelEvent.Unload`).
        if self.remote.is_none() {
            self.native_levels_closing();
        }
        if let Some(context) = self.native_lifecycle.world_close_final() {
            self.native_world_lifecycle(&context, WorldPhase::WorldCloseFinal, "");
        }
        self.attachment_indexes.clear();
        self.passages.retire_world();
        self.resident_entities.table.get_mut().reset();
        super::cauldron_interactions::clear();
        self.clear_composter_randoms();
        mod_api::__private::clear_game_parts();
    }

    pub(super) fn native_lifecycle_server_tick_start(&mut self, world_running: bool) {
        if let Some((context, phase)) = self.native_lifecycle.server_tick_start(world_running) {
            self.native_lifecycle.read_scope = true;
            self.native_server_tick(&context, phase);
            // Tasks come due after the start phase, still inside the tick's read scope. One that
            // panics does not drop the others; its panic goes on once the scope is restored.
            let panic = run_all(
                self,
                |game| game.native_lifecycle.tasks.pop_due(),
                |game, (id, owner, token)| game.native_scheduled_task(id, owner, token),
            );
            self.native_lifecycle.read_scope = false;
            if let Some(payload) = panic {
                std::panic::resume_unwind(payload);
            }
        }
    }

    /// `schedule_task`: a delayed call to `scheduled_task` for the mod whose hook is running.
    pub(in crate::game) fn schedule_task(
        &mut self,
        delay_ticks: u32,
        token: u64,
    ) -> Result<TaskId, ScheduleError> {
        let owner = mod_api::content::crash::current_native_mod();
        self.native_lifecycle
            .tasks
            .schedule(owner, delay_ticks, token)
    }

    /// `cancel_task`: withdraws a task the running mod scheduled.
    pub(in crate::game) fn cancel_task(&mut self, id: TaskId) -> bool {
        let owner = mod_api::content::crash::current_native_mod();
        self.native_lifecycle.tasks.cancel(owner, id)
    }

    /// An errored tick returned at `result?` before this site.
    pub(super) fn native_lifecycle_server_tick_end(&mut self, paused: bool) {
        if let Some((context, phase)) = self.native_lifecycle.server_tick_end(paused) {
            self.native_lifecycle.read_scope = true;
            self.native_server_tick(&context, phase);
            self.native_lifecycle.read_scope = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_epochs_never_emit_server_lifecycle_events() {
        let mut lifecycle = NativeLifecycle::default();
        lifecycle.begin_open(Authority::RemoteClient);
        let epoch = lifecycle.epoch().unwrap();
        assert!(lifecycle.world_open_ready().is_none());
        assert_eq!(lifecycle.epoch(), Some(epoch));
        assert!(lifecycle.server_tick_start(true).is_none());
        assert!(lifecycle.server_tick_end(false).is_none());
        assert!(!lifecycle.permits_read(&lifecycle.attempt.unwrap().context()));
        lifecycle.begin_close();
        assert!(lifecycle.world_close_final().is_none());
        assert!(lifecycle.epoch().is_none());

        lifecycle.begin_open(Authority::RemoteClient);
        assert_ne!(lifecycle.epoch(), Some(epoch));
        assert!(lifecycle.world_open_failed().is_none());
        assert!(lifecycle.epoch().is_none());

        lifecycle.begin_open(Authority::RemoteClient);
        lifecycle.world_open_ready();
        assert!(lifecycle.world_close_failed().is_none());
        assert!(lifecycle.epoch().is_none());
    }

    #[test]
    fn frozen_steps_are_observed_without_consuming_them() {
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        let mut clock = horizon_client::clock::FixedStep::default();
        clock.set_frozen(true);
        clock.set_frozen_ticks_to_run(4);
        let mut observed = 0;
        for _ in 0..6 {
            let running = clock.world_runs();
            let (_, start) = lifecycle.server_tick_start(running).unwrap();
            let (_, end) = lifecycle.server_tick_end(false).unwrap();
            assert_eq!(
                start,
                ServerTickPhase::ServerTickStart {
                    world_running: running
                }
            );
            assert_eq!(
                end,
                ServerTickPhase::ServerTickEnd {
                    world_running: running
                }
            );
            observed += usize::from(running);
        }
        assert_eq!(observed, 4);
    }

    /// A task a mod schedules from the tick's start hook (after `server_tick_start` moved "now")
    /// fires on a later tick: delay 0 and 1 on the next, delay N on the Nth after this one. A
    /// paused pass moves nothing, and closing the world drops what waits.
    #[test]
    fn tasks_scheduled_from_the_start_hook_fire_on_a_later_tick() {
        let owner = Some("mymod");
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        let mut fired = Vec::new();
        for tick in 1..=6u64 {
            assert!(lifecycle.server_tick_start(true).is_some());
            // The start hook runs here: it schedules on the first tick only.
            if tick == 1 {
                for (delay, token) in [(0, 100), (1, 101), (3, 103)] {
                    lifecycle.tasks.schedule(owner, delay, token).unwrap();
                }
            }
            while let Some((_, _, token)) = lifecycle.tasks.pop_due() {
                fired.push((tick, token));
            }
            lifecycle.server_tick_end(false);
        }
        assert_eq!(fired, [(2, 100), (2, 101), (4, 103)]);

        lifecycle.tasks.schedule(owner, 1, 1).unwrap();
        assert!(lifecycle.server_tick_end(true).is_none());
        assert!(lifecycle.tasks.pop_due().is_none());
        lifecycle.begin_close();
        lifecycle.world_close_final();
        lifecycle.begin_open(Authority::IntegratedServer);
        lifecycle.world_open_ready();
        lifecycle.server_tick_start(true);
        assert!(lifecycle.tasks.pop_due().is_none());
    }

    /// A lifecycle mid-open on the server thread.
    fn opening() -> NativeLifecycle {
        let mut lifecycle = NativeLifecycle::default();
        lifecycle.begin_open(Authority::IntegratedServer);
        lifecycle
    }

    #[test]
    fn block_reads_require_the_current_hook_scope() {
        let mut life = NativeLifecycle::default();
        life.begin_open(Authority::IntegratedServer);
        let ready = life.world_open_ready().unwrap();
        assert!(!life.permits_read(&ready));
        let (tick, _) = life.server_tick_start(true).unwrap();
        life.read_scope = true;
        assert!(life.permits_read(&tick));
        assert!(!life.permits_read(&ready));
        life.read_scope = false;
        assert!(!life.permits_read(&tick));
        life.server_tick_end(false).unwrap();
        life.begin_close();
        life.world_close_final().unwrap();
        life.begin_open(Authority::IntegratedServer);
        life.world_open_ready().unwrap();
        life.read_scope = true;
        assert!(!life.permits_read(&tick));
    }

    #[test]
    fn world_ready_fires_once_per_epoch() {
        let mut lifecycle = opening();
        let context = lifecycle.world_open_ready().expect("the first ready fires");
        assert_eq!(context.authority, Authority::IntegratedServer);
        assert!(context.epoch.get() >= 1, "the epoch is globally allocated");
        assert!(lifecycle.world_open_ready().is_none(), "at most once");
    }

    #[test]
    fn a_failed_open_invalidates_and_a_retry_is_fresh() {
        let mut lifecycle = opening();
        let context = lifecycle.world_open_failed().expect("the failure fires");
        let failed_epoch = context.epoch;
        assert!(lifecycle.world_open_ready().is_none(), "the epoch is gone");
        assert!(lifecycle.server_tick_start(true).is_none());
        lifecycle.begin_open(Authority::IntegratedServer);
        let context = lifecycle.world_open_ready().expect("the retry opens fresh");
        assert!(context.epoch.get() > failed_epoch.get(), "a fresh epoch");
    }

    #[test]
    fn close_final_fires_once_and_retires_the_epoch() {
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        lifecycle.begin_close();
        let context = lifecycle.world_close_final().expect("the close fires");
        assert!(context.epoch.get() >= 1);
        assert!(
            lifecycle.world_close_final().is_none(),
            "a repeated close request is a no-op"
        );
        assert!(lifecycle.server_tick_start(true).is_none());
        assert!(lifecycle.world_open_ready().is_none());
    }

    #[test]
    fn a_close_during_opening_still_closes_once() {
        let mut lifecycle = opening();
        lifecycle.begin_close();
        assert!(lifecycle.world_close_final().is_some());
    }

    #[test]
    fn close_failure_retires_only_ready_epochs() {
        let mut lifecycle = opening();
        assert!(lifecycle.world_close_failed().is_none());
        lifecycle.world_open_ready();
        lifecycle.begin_close();
        assert!(lifecycle.world_close_failed().is_some());
        assert!(lifecycle.world_close_failed().is_none());
        assert!(lifecycle.world_close_final().is_none());
    }
}
