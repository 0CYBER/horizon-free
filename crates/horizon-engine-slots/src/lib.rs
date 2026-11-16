//! Engine-owned registration sockets, point records and typed point dispatch.
//! The platform installs providers and retains listener ordering, guards and replacement authority.
extern crate mod_vocab as mod_api;

use std::sync::Mutex;

// Process state: the provider and final registration boundary belong to this game process.
static LIFECYCLE: Lifecycle = Lifecycle::new();

/// A refused load-time change to the engine's registration boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Sealed,
    AlreadyInstalled,
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Sealed => "engine registration is sealed",
            Self::AlreadyInstalled => "an engine seal provider is already installed",
        })
    }
}
impl std::error::Error for Refusal {}

enum State {
    Open(Option<fn()>),
    Sealing,
    Sealed,
}
struct Lifecycle(Mutex<State>);
impl Lifecycle {
    const fn new() -> Self {
        Self(Mutex::new(State::Open(None)))
    }
    fn install(&self, callback: fn()) -> Result<(), Refusal> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match &mut *state {
            State::Open(provider @ None) => {
                *provider = Some(callback);
                Ok(())
            }
            State::Open(Some(_)) => Err(Refusal::AlreadyInstalled),
            State::Sealing | State::Sealed => Err(Refusal::Sealed),
        }
    }
    fn seal(&self) -> Result<(), Refusal> {
        let callback = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            let State::Open(callback) = *state else {
                return Err(Refusal::Sealed);
            };
            *state = State::Sealing;
            callback
        };
        // The provider finishes listener registration here, before the final boundary closes.
        if let Some(callback) = callback {
            callback();
        }
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = State::Sealed;
        Ok(())
    }
}

/// Install the provider before engine startup. Installation during or after sealing, or after
/// another provider was installed, is refused.
pub fn install_seal(callback: fn()) -> Result<(), Refusal> {
    LIFECYCLE.install(callback)
}
/// Finish listener registration once, including when no provider was loaded.
/// Repeat, concurrent and re-entrant calls return [`Refusal::Sealed`] without
/// invoking the provider. Only the successful caller waits for its callback to finish.
pub fn seal() -> Result<(), Refusal> {
    LIFECYCLE.seal()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seal_without_provider_refuses_late_installation() {
        let lifecycle = Lifecycle::new();
        assert_eq!(lifecycle.seal(), Ok(()));
        assert_eq!(
            lifecycle.install(|| panic!("late provider")),
            Err(Refusal::Sealed)
        );
        assert_eq!(lifecycle.seal(), Err(Refusal::Sealed));
    }
    #[test]
    fn engine_seal_invokes_only_the_installed_provider() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        let lifecycle = Lifecycle::new();
        assert_eq!(
            lifecycle.install(|| {
                CALLS.fetch_add(1, Ordering::Relaxed);
            }),
            Ok(())
        );
        assert_eq!(
            lifecycle.install(|| panic!("second provider")),
            Err(Refusal::AlreadyInstalled)
        );
        assert_eq!(lifecycle.seal(), Ok(()));
        assert_eq!(lifecycle.seal(), Err(Refusal::Sealed));
        assert_eq!(CALLS.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn repeat_seal_is_refused() {
        let lifecycle = Lifecycle::new();
        assert_eq!(lifecycle.seal(), Ok(()));
        assert_eq!(lifecycle.seal(), Err(Refusal::Sealed));
    }
    #[test]
    fn reentrant_seal_and_install_are_refused_before_callback_finishes() {
        static REENTRANT: Lifecycle = Lifecycle::new();
        REENTRANT
            .install(|| {
                assert_eq!(REENTRANT.seal(), Err(Refusal::Sealed));
                assert_eq!(REENTRANT.install(|| {}), Err(Refusal::Sealed));
            })
            .unwrap();
        assert_eq!(REENTRANT.seal(), Ok(()));
        assert_eq!(REENTRANT.seal(), Err(Refusal::Sealed));
    }
}

mod platform;
pub use platform::{Platform, PlatformRefusal, install_platform, platform};

include!(concat!(env!("OUT_DIR"), "/engine_points.rs"));

/// The game's point table, set once by the game before any listened point is published.
static POINT_TABLE: std::sync::OnceLock<&'static mod_vocab::__private::points::Table> =
    std::sync::OnceLock::new();

/// Install the game's point table before publishing any listened point.
///
/// This is set-once: a second installation returns [`Refusal::AlreadyInstalled`].
/// Direct typed dispatch before installation panics; quiet macro sites skip dispatch
/// while their activity flag is unset.
///
/// # Safety
/// For each catalog index, every typed listener the table holds must be a valid
/// function pointer with that point's generated host and record signature.
pub unsafe fn install_points(
    table: &'static mod_vocab::__private::points::Table,
) -> Result<(), Refusal> {
    POINT_TABLE
        .set(table)
        .map_err(|_| Refusal::AlreadyInstalled)
}

/// Engine-side point activity and dispatch boundary.
#[doc(hidden)]
pub mod __private {
    use super::*;
    pub use mod_vocab::__private::points::{ApiKind, PointHost, cold};
    /// A quiet point costs one acquire load.
    pub static POINTS: [std::sync::atomic::AtomicBool; POINT_COUNT] =
        [const { std::sync::atomic::AtomicBool::new(false) }; POINT_COUNT];
    /// Dispatch through the installed point table without owning listener policy.
    ///
    /// Panics if [`install_points`] has not installed a table. Quiet macro sites
    /// check their activity flag and do not call this function before publication.
    pub fn fire_api(place: PointId, call: &mut dyn FnMut(ApiKind)) {
        POINT_TABLE
            .get()
            .expect("point provider must be installed before publication")
            .fire_api(place as usize, call);
    }
}
