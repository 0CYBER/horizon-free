//! Crash attribution vocabulary and engine-owned instrumentation entry points.
pub use crate::dispatch_scope::ModGuard;
use crate::dispatch_scope::backend;
use std::borrow::Cow;
use std::cell::Cell;
// Frame identity is part of crash ownership: converted-package scopes must be
// hidden by nested native calls and must expire when their owning frame exits.
#[derive(Clone, Copy)]
struct CrashOwner {
    call: Option<ModCall>,
    frame: u64,
    next_frame: u64,
}
thread_local! {
    static CURRENT_CRASH_OWNER: Cell<CrashOwner> = const { Cell::new(CrashOwner {
        call: None, frame: 0, next_frame: 0,
    }) };
}
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
pub fn native_frame() -> Option<u64> {
    CURRENT_CRASH_OWNER
        .try_with(|owner| {
            let frame = owner.get().frame;
            (frame != 0).then_some(frame)
        })
        .ok()
        .flatten()
}
#[inline]
pub(crate) fn push(call: ModCall) -> (u64, Option<ModCall>) {
    CURRENT_CRASH_OWNER
        .try_with(|owner| {
            let mut current = owner.get();
            let outer = (current.frame, current.call);
            current.frame = match current.next_frame.checked_add(1) {
                Some(next) => {
                    current.next_frame = next;
                    next
                }
                None => 0,
            };
            current.call = Some(call);
            owner.set(current);
            outer
        })
        .unwrap_or((0, None))
}
#[inline]
pub(crate) fn pop(frame: u64, call: Option<ModCall>) {
    let _ = CURRENT_CRASH_OWNER.try_with(|owner| {
        let mut current = owner.get();
        current.frame = frame;
        current.call = call;
        owner.set(current);
    });
}
// no wire: process-local crash attribution uses static runtime identities; not a hook payload.
/// A call into one hook of a native mod.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ModCall {
    /// The mod's `[mod] id`.
    pub id: &'static str,
    pub hook: &'static str,
    /// The part's type (`std::any::type_name`), when the call is to one of the mod's parts.
    pub part: Option<&'static str>,
}
// no wire: process-local crash attribution uses static runtime identities; not a hook payload.
/// A call into one package a native runtime hosts, beside the runtime's own [`ModCall`].
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ConvertedCall {
    /// The hosting runtime's `[mod] id`, the owner the report names first.
    pub runtime: &'static str,
    /// The package the runtime declared it loaded.
    pub package: Cow<'static, str>,
    /// The guest export being run.
    pub export: Cow<'static, str>,
    /// Where in the guest code, as the runtime names it (`Foo.tick(Foo.java:42)`).
    pub site: Cow<'static, str>,
}

// no wire: process-local crash attribution uses static runtime identities; not a hook payload.
/// The packages one native runtime declared it loaded.
///
/// A runtime that hosts converted code (a converter's wasm packages, a Java interpreter running
/// jars) builds this once from the packages it loaded and keeps it while they run. A call into
/// a package goes through the [`ConvertedPackage`] this registry hands out for an id it
/// declared, so guest code cannot name a package from another mod. The runtime's own `[mod] id`
/// stays the report's first name; a package and its call are told after it.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ConvertedPackages {
    runtime: &'static str,
    packages: Vec<Cow<'static, str>>,
}

impl ConvertedPackages {
    /// Declares `packages` as the ones `runtime` (its `[mod] id`) loaded. Ids read from
    /// manifests or module bytes are stored here. Entering clones an owned id on each call.
    pub fn new<I>(runtime: &'static str, packages: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<Cow<'static, str>>,
    {
        Self {
            runtime,
            packages: packages.into_iter().map(Into::into).collect(),
        }
    }

    /// The handle for `id`, or None when this runtime did not declare that package.
    pub fn package(&self, id: &str) -> Option<ConvertedPackage<'_>> {
        self.packages
            .iter()
            .find(|package| package.as_ref() == id)
            .map(|package| ConvertedPackage {
                runtime: self.runtime,
                package,
            })
    }
}

// no wire: borrowed crash attribution entry tied to its package inventory.
/// One package a [`ConvertedPackages`] declared; only that registry makes one.
#[derive(Clone, Copy, Debug)]
// exhaustive: its fields are private, so only a registry creates it
pub struct ConvertedPackage<'a> {
    runtime: &'static str,
    package: &'a Cow<'static, str>,
}

impl ConvertedPackage<'_> {
    /// Marks this thread as running `export` of this package at `site` until the guard drops:
    /// a panic in that call names the runtime, the package, the export and the site. Nested
    /// scopes restore the one they found, on a normal return and while a panic unwinds.
    ///
    /// None when this thread is not inside a call the package's runtime owns: another native
    /// mod's hook never attributes its crash to this runtime's package.
    pub fn enter(
        &self,
        export: impl Into<Cow<'static, str>>,
        site: impl Into<Cow<'static, str>>,
    ) -> Option<ConvertedPackageGuard> {
        if current_native_mod() != Some(self.runtime) {
            return None;
        }
        let call = ConvertedCall {
            runtime: self.runtime,
            package: self.package.clone(),
            export: export.into(),
            site: site.into(),
        };
        crate::dispatch_scope::backend().and_then(|backend| (backend.enter_converted)(call))
    }
}
/// The call currently attributed to the executing thread.
pub fn current_native_call() -> Option<ModCall> {
    CURRENT_CRASH_OWNER
        .try_with(|owner| owner.get().call)
        .ok()
        .flatten()
}
/// The current native mod identity.
pub fn current_native_mod() -> Option<&'static str> {
    current_native_call().map(|call| call.id)
}
/// Enter an instrumented native call, or an inert scope without a backend.
#[inline]
pub fn enter_native_mod(call: ModCall) -> ModGuard {
    crate::dispatch_scope::enter(call)
}
/// The current hosted package call.
pub fn current_converted_call() -> Option<ConvertedCall> {
    backend().and_then(|b| (b.current_converted)())
}
/// Set a crash report detail when instrumentation is installed.
pub fn set_detail(key: &'static str, value: impl Into<String>) {
    if let Some(b) = backend() {
        (b.set_detail)(key, value.into());
    }
}
/// Clear a crash report detail.
pub fn clear_detail(key: &'static str) {
    if let Some(b) = backend() {
        (b.clear_detail)(key);
    }
}
/// Snapshot the current crash report details.
pub fn details() -> Vec<(&'static str, String)> {
    backend().map(|b| (b.details)()).unwrap_or_default()
}
/// An allocation-free hosted package scope, restored by its owning backend.
#[must_use]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
// no wire: process-local scope ownership or callbacks cannot cross a mod boundary.
pub struct ConvertedPackageGuard {
    leave: fn(u64, Option<ConvertedScope>),
    token: u64,
    outer: Option<ConvertedScope>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
/// Per-call hosted attribution data, not process state.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
// no wire: process-local scope ownership or callbacks cannot cross a mod boundary.
pub struct ConvertedScope {
    pub call: ConvertedCall,
    pub frame: u64,
    pub token: u64,
}
impl ConvertedPackageGuard {
    /// Construct a guard for a hosted attribution scope.
    // hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
    #[doc(hidden)]
    pub fn new(
        leave: fn(u64, Option<ConvertedScope>),
        token: u64,
        outer: Option<ConvertedScope>,
    ) -> Self {
        Self {
            leave,
            token,
            outer,
            _thread: std::marker::PhantomData,
        }
    }
}
impl Drop for ConvertedPackageGuard {
    fn drop(&mut self) {
        (self.leave)(self.token, self.outer.take());
    }
}
