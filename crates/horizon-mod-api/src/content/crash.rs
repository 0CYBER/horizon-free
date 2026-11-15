//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
pub use mod_vocab::content::crash::{
    ConvertedCall, ConvertedPackage, ConvertedPackageGuard, ConvertedPackages, ConvertedScope,
    ModCall, ModGuard,
};
use std::cell::{Cell, RefCell};
use std::sync::Mutex;

thread_local! {
    /// The native mod hook this thread is inside, for the panic hook to name.

    /// The converted package this thread is executing, for the panic hook to name.
    static IN_CONVERTED_PACKAGE: RefCell<Option<ConvertedScope>> = const { RefCell::new(None) };
    static NEXT_SCOPE: Cell<u64> = const { Cell::new(0) };
    // Only unusual, out-of-order drops allocate; ordinary scope entry does not.
    static ENDED_SCOPES: RefCell<Vec<(u64, Option<ConvertedScope>)>> = const { RefCell::new(Vec::new()) };
}

/// The native mod hook this thread is inside, if any.
pub fn current_native_call() -> Option<ModCall> {
    mod_vocab::content::crash::current_native_call()
}
/// What the game is doing, set as it goes: graphics adapter, settings, packs, world.
static DETAILS: Mutex<Vec<(&'static str, String)>> = Mutex::new(Vec::new());
/// Marks this thread as inside `call` until the guard drops, which also happens while a panic
/// unwinds. A hook called from a hook is named by the inner call.
pub fn enter_native_mod(call: ModCall) -> ModGuard {
    let _ = ensure_installed();
    mod_vocab::content::crash::enter_native_mod(call)
}
fn enter_native(factory: mod_vocab::dispatch_scope::ModGuardFactory, call: ModCall) -> ModGuard {
    factory.finish(mod_vocab::dispatch_scope::State {
        timing: crate::__private::profiler::enter(call).map(|guard| guard.into_timing()),
    })
}
/// The `[mod] id` of the native mod this thread is inside a hook of, if any.
pub fn current_native_mod() -> Option<&'static str> {
    current_native_call().map(|call| call.id)
}

/// The converted package this thread is executing, if any.
pub fn current_converted_call() -> Option<ConvertedCall> {
    IN_CONVERTED_PACKAGE
        .try_with(|current| {
            let current = current.borrow();
            let scope = current.as_ref()?;
            let frame = mod_vocab::content::crash::native_frame()?;
            (scope.frame == frame).then(|| scope.call.clone())
        })
        .ok()
        .flatten()
}

fn enter_converted(call: ConvertedCall) -> Option<ConvertedPackageGuard> {
    if current_native_mod() != Some(call.runtime) {
        return None;
    }
    let frame = mod_vocab::content::crash::native_frame()?;
    let token = NEXT_SCOPE
        .try_with(|c| {
            let next = c.get().checked_add(1)?;
            c.set(next);
            Some(next)
        })
        .ok()
        .flatten()?;
    let scope = ConvertedScope { call, frame, token };
    let outer = IN_CONVERTED_PACKAGE
        .try_with(|current| current.replace(Some(scope)))
        .ok()?;
    Some(ConvertedPackageGuard::new(leave_converted, token, outer))
}

fn leave_converted(token: u64, mut outer: Option<ConvertedScope>) {
    let _ = IN_CONVERTED_PACKAGE.try_with(|current| {
        let mut current = current.borrow_mut();
        if current.as_ref().is_some_and(|s| s.token == token) {
            let mut outer = outer.take();
            let restored = ENDED_SCOPES.try_with(|ended| {
                let mut ended = ended.borrow_mut();
                while let Some(index) = outer
                    .as_ref()
                    .and_then(|scope| ended.iter().position(|(token, _)| *token == scope.token))
                {
                    outer = ended.swap_remove(index).1;
                }
            });
            *current = if restored.is_ok() { outer } else { None };
        } else {
            let _ = ENDED_SCOPES.try_with(|ended| ended.borrow_mut().push((token, outer.take())));
        }
    });
}

fn leave_native(state: mod_vocab::dispatch_scope::State) {
    if let Some(timing) = state.timing {
        crate::__private::profiler::leave(timing);
    }
}
/// Sets one line of the report's details (replacing the last value under `key`).
pub fn set_detail(key: &'static str, value: impl Into<String>) {
    let mut details = DETAILS.lock().unwrap_or_else(|e| e.into_inner());
    let value = value.into();
    match details.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = value,
        None => details.push((key, value)),
    }
}
pub fn clear_detail(key: &'static str) {
    DETAILS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(k, _)| *k != key);
}
pub fn details() -> Vec<(&'static str, String)> {
    DETAILS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub(crate) fn ensure_installed() -> Result<(), &'static str> {
    static INSTALLED: std::sync::OnceLock<Result<(), &'static str>> = std::sync::OnceLock::new();
    *INSTALLED.get_or_init(|| {
        mod_vocab::dispatch_scope::install(mod_vocab::dispatch_scope::Backend {
            unresolved_fluids: crate::fluid_state::ACCESS,
            registration: &crate::parts::REGISTRATION_ACCESS,
            diagnostics: &crate::parts::DIAGNOSTICS,
            enter: enter_native,
            leave: leave_native,
            profiler_enabled: &crate::__private::profiler::ENABLED,
            current_converted: current_converted_call,
            enter_converted,
            set_detail: |key, value| set_detail(key, value),
            clear_detail,
            details,
            current_phase: crate::load_state::current_phase,
            phase_state: crate::load_state::phase_snapshot,
            set_phase: crate::load_state::set_phase,
            packages: crate::load_state::current_packages,
            registration_data: crate::load_state::registration_data,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_guard_is_thread_affine() {
        trait Ambiguous<A> {
            fn check() {}
        }
        impl<T: ?Sized> Ambiguous<()> for T {}
        impl<T: ?Sized + Send> Ambiguous<u8> for T {}
        let _ = <ConvertedPackageGuard as Ambiguous<_>>::check;
        let _ = <ModGuard as Ambiguous<_>>::check;
    }

    #[test]
    fn crash_retained_scope_cannot_reach_later_frame() {
        let packages = ConvertedPackages::new("runtime", ["guest"]);
        let native = runtime("first");
        let scope = packages
            .package("guest")
            .unwrap()
            .enter("run", "site")
            .unwrap();
        drop(native);
        let _later = runtime("second");
        assert_eq!(current_converted_call(), None);
        drop(scope);
    }

    #[test]
    fn crash_nested_native_frame_hides_outer_scope() {
        let packages = ConvertedPackages::new("runtime", ["guest"]);
        let _native = runtime("first");
        let _scope = packages
            .package("guest")
            .unwrap()
            .enter("run", "site")
            .unwrap();
        let inner = runtime("second");
        assert_eq!(current_converted_call(), None);
        drop(inner);
        assert!(current_converted_call().is_some());
    }

    #[test]
    fn crash_out_of_order_scope_drop_does_not_resurrect() {
        let packages = ConvertedPackages::new("runtime", ["guest"]);
        let _native = runtime("first");
        let package = packages.package("guest").unwrap();
        let outer = package.enter("outer", "site").unwrap();
        let inner = package.enter("inner", "site").unwrap();
        drop(outer);
        assert_eq!(current_converted_call().unwrap().export, "inner");
        drop(inner);
        assert_eq!(current_converted_call(), None);
    }

    #[test]
    fn crash_three_scopes_restore_live_ancestor() {
        let packages = ConvertedPackages::new("runtime", ["guest"]);
        let _native = runtime("dispatch");
        let package = packages.package("guest").unwrap();
        for order in [[1, 2, 0], [1, 0, 2], [0, 1, 2]] {
            let mut guards = ["A", "B", "C"].map(|name| package.enter(name, "site"));
            drop(guards[order[0]].take());
            assert_eq!(current_converted_call().unwrap().export, "C");
            drop(guards[order[1]].take());
            if order == [1, 2, 0] {
                assert_eq!(current_converted_call().unwrap().export, "A");
            } else {
                assert_eq!(current_converted_call().unwrap().export, "C");
            }
            drop(guards[order[2]].take());
            assert_eq!(current_converted_call(), None);
        }
    }

    #[test]
    fn crash_scope_reader_rejects_exited_native_frame() {
        let packages = ConvertedPackages::new("runtime", ["guest"]);
        let native = runtime("first");
        let _scope = packages
            .package("guest")
            .unwrap()
            .enter("run", "site")
            .unwrap();
        drop(native);
        assert_eq!(current_converted_call(), None);
    }

    #[test]
    fn crash_tls_teardown_is_non_panicking() {
        struct Last {
            native: Option<ModGuard>,
            scope: Option<ConvertedPackageGuard>,
        }
        impl Drop for Last {
            fn drop(&mut self) {
                drop(self.scope.take());
                assert_eq!(current_converted_call(), None);
                assert_eq!(current_native_mod(), Some("runtime"));
                let packages = ConvertedPackages::new("runtime", ["guest"]);
                assert!(
                    packages
                        .package("guest")
                        .unwrap()
                        .enter("run", "site")
                        .is_none()
                );
                drop(self.native.take());
            }
        }
        thread_local! {
            static LAST: RefCell<Last> = const { RefCell::new(Last { native: None, scope: None }) };
        }
        std::thread::spawn(|| {
            // Assumes reverse TLS destructor registration order: LAST is initialized
            // before the converted scope TLS, so its guards outlive that TLS.
            LAST.with(|_| {});
            let native = runtime("first");
            let packages = ConvertedPackages::new("runtime", ["guest"]);
            let scope = packages
                .package("guest")
                .unwrap()
                .enter("run", "site")
                .unwrap();
            LAST.with(|last| {
                let mut last = last.borrow_mut();
                last.native = Some(native);
                last.scope = Some(scope);
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn crash_out_of_order_tombstones_are_reclaimed() {
        let _native = runtime("dispatch");
        let packages = ConvertedPackages::new("runtime", ["guest"]);
        let package = packages.package("guest").unwrap();
        for _ in 0..128 {
            let a = package.enter("a", "site").unwrap();
            let b = package.enter("b", "site").unwrap();
            let c = package.enter("c", "site").unwrap();
            drop(b);
            drop(a);
            drop(c);
            ENDED_SCOPES.with(|ended| assert!(ended.borrow().is_empty()));
        }
    }

    fn runtime(hook: &'static str) -> ModGuard {
        enter_native_mod(ModCall {
            id: "runtime",
            hook,
            part: None,
        })
    }

    #[test]
    fn a_declared_package_scopes_a_call_to_it() {
        let _runtime = runtime("dispatch");
        let packages = ConvertedPackages::new("runtime", ["example:mod", "example:data"]);
        let package = packages.package("example:mod").unwrap();
        let call = package
            .enter("tick", "com.example.Foo.tick(Foo.java:42)")
            .unwrap();
        let scoped = current_converted_call().unwrap();
        assert_eq!(scoped.runtime, "runtime");
        assert_eq!(scoped.package, "example:mod");
        assert_eq!(scoped.export, "tick");
        assert_eq!(scoped.site, "com.example.Foo.tick(Foo.java:42)");
        drop(call);
        assert_eq!(current_converted_call(), None);
    }

    #[test]
    fn an_undeclared_package_is_rejected() {
        let packages = ConvertedPackages::new("runtime", ["example:mod"]);
        assert!(packages.package("example:other").is_none());
    }

    #[test]
    fn another_mods_call_cannot_enter_this_runtimes_package() {
        let packages = ConvertedPackages::new("runtime", ["example:mod"]);
        let package = packages.package("example:mod").unwrap();
        let _other = enter_native_mod(ModCall {
            id: "other",
            hook: "tick",
            part: None,
        });
        assert!(package.enter("tick", "Foo.tick").is_none());
        assert_eq!(current_converted_call(), None);
        // And with no native call at all.
        drop(_other);
        assert!(package.enter("tick", "Foo.tick").is_none());
    }

    #[test]
    fn nested_scopes_restore_the_one_before() {
        let _runtime = runtime("dispatch");
        let packages = ConvertedPackages::new("runtime", ["example:outer", "example:inner"]);
        let outer = packages.package("example:outer").unwrap();
        let inner = packages.package("example:inner").unwrap();
        let _outer = outer.enter("run", "Outer.run(Outer.java:1)").unwrap();
        let _inner = inner.enter("draw", "Inner.draw(Inner.java:2)").unwrap();
        assert_eq!(current_converted_call().unwrap().package, "example:inner");
        drop(_inner);
        assert_eq!(current_converted_call().unwrap().package, "example:outer");
        drop(_outer);
        assert_eq!(current_converted_call(), None);
    }

    #[test]
    fn a_scope_unwound_by_a_panic_restores_the_one_before() {
        let _runtime = runtime("dispatch");
        let packages = ConvertedPackages::new("runtime", ["example:outer", "example:inner"]);
        let outer = packages.package("example:outer").unwrap();
        let inner = packages.package("example:inner").unwrap();
        let _outer = outer.enter("run", "Outer.run(Outer.java:1)").unwrap();
        // The panic is the guest's, not ours: nothing here catches it, so it unwinds past
        // the scope, which goes back to the outer package on the way out.
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _inner = inner.enter("draw", "Inner.draw(Inner.java:2)").unwrap();
            panic!("guest panic");
        }));
        assert!(panic.is_err(), "a fatal panic is not swallowed");
        assert_eq!(current_converted_call().unwrap().package, "example:outer");
        assert_eq!(current_native_mod(), Some("runtime"));
    }
}
