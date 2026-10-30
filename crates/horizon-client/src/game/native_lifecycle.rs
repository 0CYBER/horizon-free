//! Read-only world lifecycle observations for native mods on authoritative
//! servers. The deprecated in-frame path delivers no lifecycle events.

use std::sync::atomic::{AtomicU64, Ordering};

use super::{Game, execution_role::ExecutionRole};

/// Epoch allocation spans `Game` instances (a new server `Game` per
/// launch), so the counter is process-wide.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// Which `Game` observes the phase; a mirror `Game` never fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeAuthority {
    LocalAuthoritative,
    IntegratedServer,
    DedicatedServer,
    RemoteClient,
}

/// One open attempt of one world; it rejects a previous open's commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NativeWorldEpoch(u64);

/// Internal state for deciding which events can fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenState {
    Opening,
    Ready,
    Closing,
}

/// The world phases (`native_world_lifecycle`'s `phase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The phase names are the hook log's stable vocabulary (the Debug text
/// the mixins write); keep them despite the shared prefix.
#[allow(clippy::enum_variant_names)]
pub(super) enum WorldPhase {
    WorldOpenReady,
    /// The open failed; `error_text` is the engine's own error text.
    WorldOpenFailed,
    WorldCloseFinal,
    /// The server loop or final save failed; `error_text` carries the error.
    WorldCloseFailed,
}

/// Context for every world lifecycle phase.
/// `authority` is the server role. LocalAuthoritative and RemoteClient never
/// fire today. `epoch` identifies the open attempt, including failed opens;
/// the same epoch identifies its ready, final-close, or failed-close event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NativeLifecycleContext {
    pub(super) authority: NativeAuthority,
    pub(super) epoch: NativeWorldEpoch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OpenAttempt {
    authority: NativeAuthority,
    epoch: u64,
    session: OpenState,
}

impl OpenAttempt {
    fn context(&self) -> NativeLifecycleContext {
        NativeLifecycleContext {
            authority: self.authority,
            epoch: NativeWorldEpoch(self.epoch),
        }
    }
}

/// The lifecycle bookkeeping: the current attempt's epoch and session phase.
/// It only decides whether a phase fires; every hook body is empty, so with no
/// native mods nothing changes.
#[derive(Default)]
pub(super) struct NativeLifecycle {
    attempt: Option<OpenAttempt>,
}

impl NativeLifecycle {
    /// The open starts: a fresh process-wide epoch, never reused.
    pub(super) fn begin_open(&mut self, authority: NativeAuthority) {
        let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
        self.attempt = Some(OpenAttempt {
            authority,
            epoch,
            session: OpenState::Opening,
        });
    }

    pub(super) fn world_open_ready(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Opening {
            return None;
        }
        attempt.session = OpenState::Ready;
        Some(attempt.context())
    }

    /// `WorldOpenFailed`: only while opening; the attempt is invalidated.
    pub(super) fn world_open_failed(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Opening {
            return None;
        }
        let context = attempt.context();
        self.attempt = None;
        Some(context)
    }

    pub(super) fn begin_close(&mut self) {
        if let Some(attempt) = self.attempt.as_mut() {
            attempt.session = OpenState::Closing;
        }
    }

    /// A failure after readiness retires the epoch, including during shutdown.
    pub(super) fn world_close_failed(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_ref()?;
        if attempt.session == OpenState::Opening {
            return None;
        }
        let context = attempt.context();
        self.attempt = None;
        Some(context)
    }

    /// `WorldCloseFinal`: at most once per epoch; the epoch retires.
    pub(super) fn world_close_final(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Closing {
            return None;
        }
        let context = attempt.context();
        self.attempt = None;
        Some(context)
    }
}

impl Game<'_> {
    /// The role is fixed at construction; a mirror `Game` is remote whatever it is.
    fn native_lifecycle_authority(&self) -> NativeAuthority {
        if self.remote.is_some() {
            return NativeAuthority::RemoteClient;
        }
        match self.options.execution_role {
            ExecutionRole::Client => NativeAuthority::LocalAuthoritative,
            ExecutionRole::IntegratedServer => NativeAuthority::IntegratedServer,
            ExecutionRole::DedicatedServer => NativeAuthority::DedicatedServer,
        }
    }

    pub(super) fn native_lifecycle_open_started(&mut self) {
        self.native_lifecycle
            .begin_open(self.native_lifecycle_authority());
    }

    pub(super) fn native_lifecycle_world_ready(&mut self) {
        if let Some(context) = self.native_lifecycle.world_open_ready() {
            self.native_world_lifecycle(&context, WorldPhase::WorldOpenReady, "");
        }
    }

    /// `error_text` is the engine's own error text, unchanged.
    pub(super) fn native_lifecycle_world_failed(&mut self, error_text: &str) {
        if let Some(context) = self.native_lifecycle.world_open_failed() {
            self.native_world_lifecycle(&context, WorldPhase::WorldOpenFailed, error_text);
        }
    }

    /// Report a server-loop or shutdown failure once and retire its epoch.
    pub(super) fn native_lifecycle_close_failed(&mut self, error_text: &str) {
        if let Some(context) = self.native_lifecycle.world_close_failed() {
            self.native_world_lifecycle(&context, WorldPhase::WorldCloseFailed, error_text);
        }
    }

    /// After the final save, before the lock release. Mirrors never fire.
    pub(super) fn native_lifecycle_world_closed(&mut self) {
        if self.remote.is_some() {
            return;
        }
        if let Some(context) = self.native_lifecycle.world_close_final() {
            self.native_world_lifecycle(&context, WorldPhase::WorldCloseFinal, "");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lifecycle mid-open on the server thread.
    fn opening() -> NativeLifecycle {
        let mut lifecycle = NativeLifecycle::default();
        lifecycle.begin_open(NativeAuthority::IntegratedServer);
        lifecycle
    }

    #[test]
    fn world_ready_fires_once_per_epoch() {
        let mut lifecycle = opening();
        let context = lifecycle.world_open_ready().expect("the first ready fires");
        assert_eq!(context.authority, NativeAuthority::IntegratedServer);
        assert!(context.epoch.0 >= 1, "the epoch is globally allocated");
        assert!(lifecycle.world_open_ready().is_none(), "at most once");
    }

    #[test]
    fn a_failed_open_invalidates_and_a_retry_is_fresh() {
        let mut lifecycle = opening();
        let context = lifecycle.world_open_failed().expect("the failure fires");
        let failed_epoch = context.epoch;
        assert!(lifecycle.world_open_ready().is_none(), "the epoch is gone");
        lifecycle.begin_open(NativeAuthority::IntegratedServer);
        let context = lifecycle.world_open_ready().expect("the retry opens fresh");
        assert!(context.epoch.0 > failed_epoch.0, "a fresh epoch");
    }

    #[test]
    fn close_final_fires_once_and_retires_the_epoch() {
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        lifecycle.begin_close();
        let context = lifecycle.world_close_final().expect("the close fires");
        assert!(context.epoch.0 >= 1);
        assert!(
            lifecycle.world_close_final().is_none(),
            "a repeated close request is a no-op"
        );
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

    #[test]
    fn dedicated_open_and_close_sites_bound_the_server_lifecycle() {
        let source = include_str!("dedicated.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        let open = source
            .find("self.native_lifecycle_open_started();")
            .unwrap();
        let load = source.find("self.load_world(WorldRequest::new(").unwrap();
        let ready = source.find("self.native_lifecycle_world_ready();").unwrap();
        let tick_loop = source.find("self.tick_server_timed(paused)?;").unwrap();
        assert!(open < load && load < ready && ready < tick_loop);

        let stop = source.split("fn stop_dedicated").nth(1).unwrap();
        let begin = stop.find("self.native_lifecycle.begin_close();").unwrap();
        let save = stop.find("self.save_game()?;").unwrap();
        let close = stop.find("self.native_lifecycle_world_closed();").unwrap();
        let release = stop.find("self.lan_close(false);").unwrap();
        assert!(begin < save && save < close && close < release);
    }

    #[test]
    fn lifecycle_delivery_stays_off_the_deprecated_in_frame_path() {
        let game = include_str!("../game.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        for site in [
            "native_lifecycle_open_started()",
            "native_lifecycle_world_ready()",
            "native_lifecycle_world_closed()",
            "native_lifecycle.begin_close()",
        ] {
            assert!(!game.contains(site), "in-frame site remains: {site}");
        }
        let saving = include_str!("saving.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        assert!(!saving.contains("native_lifecycle"));

        let integrated = include_str!("integrated.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        let open = integrated
            .find("self.native_lifecycle_open_started();")
            .unwrap();
        let load = integrated
            .find("self.load_world(WorldRequest::new(")
            .unwrap();
        assert!(open < load);
    }
}
