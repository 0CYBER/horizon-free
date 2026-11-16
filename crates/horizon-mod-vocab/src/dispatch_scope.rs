//! Engine-owned dispatch instrumentation socket.
//!
//! The platform installs one backend before loading mods. A backend receives a
//! factory capability on entry: it cannot manufacture guards associated with a
//! different backend. No allocation or erased object is needed for a call.
use crate::content::crash::ModCall;
use std::sync::OnceLock;

static BACKEND: OnceLock<Option<Backend>> = OnceLock::new();

/// Platform callbacks, installed once before dispatch starts.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
// no wire: process-local scope ownership or callbacks cannot cross a mod boundary.
pub struct Backend {
    pub registration: &'static crate::__private::RegistrationAccess,
    pub diagnostics: &'static Diagnostics,
    pub unresolved_fluids: crate::__private::UnresolvedFluidAccess,
    pub enter: fn(ModGuardFactory, ModCall) -> ModGuard,
    pub leave: fn(State),
    pub profiler_enabled: &'static std::sync::atomic::AtomicBool,
    pub current_converted: fn() -> Option<crate::content::crash::ConvertedCall>,
    pub enter_converted: fn(
        crate::content::crash::ConvertedCall,
    ) -> Option<crate::content::crash::ConvertedPackageGuard>,
    pub set_detail: fn(&'static str, String),
    pub clear_detail: fn(&'static str),
    pub details: fn() -> Vec<(&'static str, String)>,
    pub current_phase: fn() -> crate::__private::ModPhase,
    pub phase_state: fn() -> crate::__private::PhaseState,
    pub set_phase: fn(crate::__private::PhaseState),
    pub packages: fn() -> std::sync::Arc<[crate::PackageInfo]>,
    pub registration_data: fn() -> Option<crate::__private::RegistrationData>,
}

// no wire: platform-owned diagnostic caches are observed through native callbacks.
/// Cold diagnostic deduplication, owned by the platform rather than vocabulary lookups.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
pub struct Diagnostics {
    pub first_broken_warning: fn(&str) -> bool,
    pub first_off_thread_warning: fn() -> bool,
}

/// Plain per-call instrumentation data. The platform must validate the pool
/// generation and slot before accessing its profiler storage.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
#[derive(Clone, Copy)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
// no wire: process-local scope ownership or callbacks cannot cross a mod boundary.
pub struct Timing {
    pub generation: usize,
    pub slot: usize,
    pub record: usize,
}

/// Platform-owned timing data bound to a scope's factory capability.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
// no wire: process-local scope ownership or callbacks cannot cross a mod boundary.
pub struct State {
    pub timing: Option<Timing>,
}

/// A capability to bind exactly one guard to the installed leave callback.
/// Its fields are private and it is neither Clone nor Copy.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
// no wire: process-local scope ownership or callbacks cannot cross a mod boundary.
pub struct ModGuardFactory {
    leave: fn(State),
    owner: (u64, Option<ModCall>),
}
impl ModGuardFactory {
    /// Consume the capability, transferring the backend's per-call state.
    pub fn finish(self, state: State) -> ModGuard {
        ModGuard {
            owner: Some(self.owner),
            _timing: state.timing.map(|timing| TimingGuard {
                leave: self.leave,
                timing,
            }),
            _thread: std::marker::PhantomData,
        }
    }
}

/// An allocation-free dispatch scope. With no instrumentation backend this is
/// inert; otherwise Drop restores attribution, including during unwinding.
#[must_use]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
// no wire: process-local scope ownership or callbacks cannot cross a mod boundary.
pub struct ModGuard {
    owner: Option<(u64, Option<ModCall>)>,
    _timing: Option<TimingGuard>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl ModGuard {
    pub(crate) const fn empty() -> Self {
        Self {
            owner: None,
            _timing: None,
            _thread: std::marker::PhantomData,
        }
    }

    #[inline]
    fn clear(&mut self) {
        if let Some((frame, call)) = self.owner.take() {
            crate::content::crash::pop(frame, call);
        }
        drop(self._timing.take());
    }
}
impl Drop for ModGuard {
    #[inline]
    fn drop(&mut self) {
        self.clear();
    }
}

struct TimingGuard {
    leave: fn(State),
    timing: Timing,
}
impl Drop for TimingGuard {
    fn drop(&mut self) {
        (self.leave)(State {
            timing: Some(self.timing),
        });
    }
}

/// Install the instrumentation backend. An already installed backend is never
/// replaced, including after an unsuccessful second registration.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
pub fn install(backend: Backend) -> Result<(), &'static str> {
    BACKEND.set(Some(backend)).map_err(|_| match BACKEND.get() {
        Some(None) => {
            "dispatch ran before platform initialization, which sealed instrumentation off"
        }
        _ => "dispatch instrumentation is already sealed",
    })
}

/// Immutable dispatch-local instrumentation selection. Resolve the socket and
/// profiler flag once, before walking listeners; nested dispatch takes its own snapshot.
#[derive(Clone, Copy)]
pub(crate) struct Scope {
    installed: bool,
    timing: Option<(fn(ModGuardFactory, ModCall) -> ModGuard, fn(State))>,
}

#[inline]
pub(crate) fn snapshot() -> Scope {
    let backend = BACKEND.get_or_init(|| None).as_ref();
    Scope {
        installed: backend.is_some(),
        timing: backend.and_then(|backend| {
            backend
                .profiler_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
                .then_some((backend.enter, backend.leave))
        }),
    }
}

#[inline]
pub(crate) fn enter(call: ModCall) -> ModGuard {
    snapshot().enter(call)
}

impl Scope {
    #[inline]
    pub(crate) fn enter(self, call: ModCall) -> ModGuard {
        let mut guard = ModGuard::empty();
        self.replace(&mut guard, call);
        guard
    }

    /// Reuse the iterator's scope slot instead of moving a full guard through
    /// `Option::take` and the iterator adapter on every listener.
    #[inline]
    pub(crate) fn replace(self, guard: &mut ModGuard, call: ModCall) {
        guard.clear();
        if self.installed {
            let owner = crate::content::crash::push(call);
            if let Some((enter, leave)) = self.timing {
                *guard = enter(ModGuardFactory { leave, owner }, call);
            } else {
                guard.owner = Some(owner);
            }
        }
    }
}

pub(crate) fn backend() -> Option<&'static Backend> {
    BACKEND.get().and_then(Option::as_ref)
}
