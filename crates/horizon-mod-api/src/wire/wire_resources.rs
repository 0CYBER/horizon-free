//! A call's resource registry. Data still uses Wire; only trait objects need the active side.
use super::{Id, WireError};
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    rc::Rc,
};

/// Resolves resource identities for the side of a synchronous boundary call.
pub trait ResourceCodec {
    fn store(&self, value: Box<dyn Any>) -> Result<Id, WireError>;
    fn load(&self, id: Id, view: TypeId) -> Result<Box<dyn Any>, WireError>;
    /// Reports a refused erased view; codecs may log this once per caller and handler.
    fn refused(&self, _handler: &str) {}
}
// exhaustive: wasm boundary plumbing that generated code builds field by field
/// A lease's scope ended, or the caller left its owning thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// no wire: local resource-scope diagnostics.
pub enum ScopeError {
    Stale,
    WrongThread,
}
impl std::fmt::Display for ScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stale => "expired resource scope",
            Self::WrongThread => "resource scope belongs to another thread",
        })
    }
}
impl std::error::Error for ScopeError {}
// exhaustive: wasm boundary plumbing that generated code builds field by field
/// A non-owning native lease. Checking it never retains the scope, codec, or pointee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// no wire: native resource-scope authority, never transferred as data.
pub struct ScopeToken {
    slot: usize,
    generation: u64,
    thread: std::thread::ThreadId,
}
impl ScopeToken {
    pub fn check(&self) -> Result<(), ScopeError> {
        if self.thread != std::thread::current().id() {
            return Err(ScopeError::WrongThread);
        }
        ACTIVE
            .try_with(|active| {
                let scopes = active.borrow();
                scopes
                    .slots
                    .get(self.slot)
                    .filter(|s| s.generation == self.generation && s.live)
                    .map(|_| ())
                    .ok_or(ScopeError::Stale)
            })
            .unwrap_or(Err(ScopeError::Stale))
    }
}
/// The nearest live runtime root, never an attestation supplied by a user codec.
pub fn borrow_token() -> Result<ScopeToken, ScopeError> {
    ACTIVE
        .try_with(|active| {
            let scopes = active.borrow();
            let mut at = scopes.current;
            while let Some(slot) = at {
                let scope = &scopes.slots[slot];
                if scope.native_root {
                    return Ok(ScopeToken {
                        slot,
                        generation: scope.generation,
                        thread: std::thread::current().id(),
                    });
                }
                at = scope.previous;
            }
            Err(ScopeError::Stale)
        })
        .unwrap_or(Err(ScopeError::Stale))
}
struct Scope {
    live: bool,
    codec: Option<Rc<dyn ResourceCodec>>,
    generation: u64,
    native_root: bool,
    previous: Option<usize>,
    next: Option<usize>,
}
#[derive(Default)]
struct Scopes {
    slots: Vec<Scope>,
    current: Option<usize>,
    free: Option<usize>,
}
thread_local! { static ACTIVE: RefCell<Scopes> = RefCell::new(Scopes::default()); }
// exhaustive: wasm boundary plumbing that generated code builds field by field
/// A detached value lease. It never installs a codec or attests a native call root.
/// Its lifetime is independent of the active resource scopes.
// no wire: owns a native resource-scope lease.
pub struct ScopeLease {
    token: ScopeToken,
    _thread: std::marker::PhantomData<Rc<()>>,
}
impl ScopeLease {
    pub fn new() -> Self {
        ensure_installed()
            .expect("platform resource backend must initialize before scope construction");
        let token = ACTIVE.with(|active| {
            let mut scopes = active.borrow_mut();
            while let Some(slot) = scopes.free {
                if scopes.slots[slot].generation < u64::MAX {
                    break;
                }
                scopes.free = scopes.slots[slot].next;
            }
            let slot = match scopes.free {
                Some(slot) => {
                    scopes.free = scopes.slots[slot].next;
                    slot
                }
                None => {
                    let slot = scopes.slots.len();
                    scopes.slots.push(Scope {
                        live: false,
                        codec: None,
                        generation: 0,
                        native_root: false,
                        previous: None,
                        next: None,
                    });
                    slot
                }
            };
            let generation = scopes.slots[slot].generation + 1;
            scopes.slots[slot] = Scope {
                live: true,
                codec: None,
                generation,
                native_root: false,
                previous: None,
                next: None,
            };
            ScopeToken {
                slot,
                generation,
                thread: std::thread::current().id(),
            }
        });
        Self {
            token,
            _thread: std::marker::PhantomData,
        }
    }
    pub fn token(&self) -> ScopeToken {
        self.token
    }
}
impl Default for ScopeLease {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for ScopeLease {
    fn drop(&mut self) {
        let _ = ACTIVE.try_with(|active| {
            let mut scopes = active.borrow_mut();
            let slot = self.token.slot;
            scopes.slots[slot].live = false;
            scopes.slots[slot].next = scopes.free;
            scopes.free = Some(slot);
        });
    }
}
// exhaustive: wasm boundary plumbing that generated code builds field by field
/// Activates a call's codec, including guest-to-host-to-guest callbacks. Dropping scopes
/// out of order unlinks them: a retained inner guard cannot revive an expired outer codec.
/// Slots are reused, so steady-state calls allocate no scope bookkeeping.
// no wire: native resource-table ownership and lifecycle state.
pub struct ResourceScope {
    slot: usize,
    generation: u64,
    // The slot belongs to this thread even though its index alone would be Send + Sync.
    _thread: std::marker::PhantomData<Rc<()>>,
}
impl ResourceScope {
    pub fn new(codec: Rc<dyn ResourceCodec>) -> Self {
        let lease = ScopeLease::new();
        let token = lease.token();
        // The linked scope now owns this same arena slot.
        std::mem::forget(lease);
        let (slot, generation) = ACTIVE.with(|active| {
            let mut scopes = active.borrow_mut();
            let slot = token.slot;
            let previous = scopes.current;
            let generation = token.generation;
            scopes.slots[slot] = Scope {
                live: true,
                codec: Some(codec),
                generation,
                native_root: false,
                previous,
                next: None,
            };
            if let Some(previous) = previous {
                scopes.slots[previous].next = Some(slot);
            }
            scopes.current = Some(slot);
            (slot, generation)
        });
        Self {
            slot,
            generation,
            _thread: std::marker::PhantomData,
        }
    }
    /// A non-owning token for this scope. Copying it cannot extend the scope's lifetime.
    pub fn token(&self) -> ScopeToken {
        ScopeToken {
            slot: self.slot,
            generation: self.generation,
            thread: std::thread::current().id(),
        }
    }
    // hidden: wasm boundary plumbing used by generated code, not a mod's API
    /// Marks the runtime's root call, from which native borrowed roots receive their lease.
    ///
    /// # Safety
    /// Runtime context roots (the codec's Cx/frame pointees) must remain at a stable address
    /// and valid until the marker drops. This marker does not cover shorter-lived encoded
    /// values: before every access, those require an independently checked private ScopeLease
    /// and must fail closed with typed Stale on expiry. Locators into mutable containers must
    /// be resolved afresh; neither lease makes element addresses stable.
    #[doc(hidden)]
    #[allow(
        unsafe_code,
        reason = "The caller owns the native resource frame lifetime."
    )]
    pub unsafe fn native_root(codec: Rc<dyn ResourceCodec>) -> Self {
        let scope = Self::new(codec);
        ACTIVE.with(|active| active.borrow_mut().slots[scope.slot].native_root = true);
        scope
    }
}
impl Drop for ResourceScope {
    fn drop(&mut self) {
        // The codec's destructor can reenter Wire. Release the TLS borrow before dropping it.
        let codec = ACTIVE.try_with(|active| {
            let mut scopes = active.borrow_mut();
            let slot = self.slot;
            let previous = scopes.slots[slot].previous;
            let next = scopes.slots[slot].next;
            if let Some(previous) = previous {
                scopes.slots[previous].next = next;
            }
            if let Some(next) = next {
                scopes.slots[next].previous = previous;
            } else {
                scopes.current = previous;
            }
            scopes.slots[slot].live = false;
            let codec = scopes.slots[slot].codec.take();
            scopes.slots[slot].next = scopes.free;
            scopes.free = Some(slot);
            codec
        });
        drop(codec);
    }
}
fn current() -> Result<Rc<dyn ResourceCodec>, WireError> {
    ACTIVE
        .with(|active| {
            let scopes = active.borrow();
            scopes
                .current
                .and_then(|slot| scopes.slots[slot].codec.clone())
        })
        .ok_or(WireError::Invalid("no resource context"))
}
pub(super) fn refused(handler: &str) {
    if let Ok(codec) = current() {
        codec.refused(handler);
    }
}
pub(super) fn store(value: Box<dyn Any>) -> Result<Id, WireError> {
    current()?.store(value)
}

pub(super) fn ensure_installed() -> Result<(), &'static str> {
    static INSTALLED: std::sync::OnceLock<Result<(), &'static str>> = std::sync::OnceLock::new();
    *INSTALLED.get_or_init(|| {
        mod_vocab::wire::install_resource_access(mod_vocab::wire::ResourceAccess {
            intern: super::static_strings::intern,
            store,
            load: |id, view| current()?.load(id, view),
            refused,
        })
    })
}

#[cfg(test)]
fn load<T: 'static>(id: Id) -> Result<T, WireError> {
    current()?
        .load(id, TypeId::of::<T>())?
        .downcast::<T>()
        .map(|value| *value)
        .map_err(|_| WireError::Invalid("wrong resource view"))
}

#[cfg(test)]
#[allow(
    unsafe_code,
    reason = "Fixtures own the native resource frame lifetime."
)]
mod tests {
    use super::*;
    struct Codec(u64);
    impl ResourceCodec for Codec {
        fn store(&self, _: Box<dyn Any>) -> Result<Id, WireError> {
            Ok(Id(self.0))
        }
        fn load(&self, _: Id, _: TypeId) -> Result<Box<dyn Any>, WireError> {
            Ok(Box::new(self.0))
        }
    }
    #[test]
    fn resource_scope_token_refuses_expiration_slot_reuse_and_foreign_thread() {
        let scope = ResourceScope::new(Rc::new(Codec(11)));
        let token = scope.token();
        assert_eq!(token.check(), Ok(()));
        assert_eq!(
            std::thread::spawn(move || token.check()).join().unwrap(),
            Err(ScopeError::WrongThread)
        );
        drop(scope);
        assert_eq!(token.check(), Err(ScopeError::Stale));
        let replacement = ResourceScope::new(Rc::new(Codec(12)));
        assert_eq!(token.check(), Err(ScopeError::Stale));
        assert_eq!(replacement.token().check(), Ok(()));
    }
    #[test]
    fn detached_lease_is_independent_of_active_scopes() {
        let lease = ScopeLease::new();
        let token = lease.token();
        assert_eq!(token.check(), Ok(()));
        assert_eq!(borrow_token(), Err(ScopeError::Stale));
        assert_eq!(
            std::thread::spawn(move || token.check()).join().unwrap(),
            Err(ScopeError::WrongThread)
        );
        let root = unsafe { ResourceScope::native_root(Rc::new(Codec(17))) };
        let marker = borrow_token().unwrap();
        let inner = ScopeLease::new();
        assert_eq!(store(Box::new(())), Ok(Id(17)));
        assert_eq!(borrow_token(), Ok(marker));
        drop(lease);
        assert_eq!(token.check(), Err(ScopeError::Stale));
        let replacement = ScopeLease::new();
        assert_eq!(replacement.token().slot, token.slot);
        assert_eq!(token.check(), Err(ScopeError::Stale));
        drop(inner);
        assert_eq!(store(Box::new(())), Ok(Id(17)));
        assert_eq!(borrow_token(), Ok(marker));
        drop(root);
        assert_eq!(replacement.token().check(), Ok(()));
        assert_eq!(borrow_token(), Err(ScopeError::Stale));
        let ordinary = ResourceScope::new(Rc::new(Codec(18)));
        let detached = ScopeLease::new();
        drop(replacement);
        drop(detached);
        assert_eq!(store(Box::new(())), Ok(Id(18)));
        assert_eq!(borrow_token(), Err(ScopeError::Stale));
        drop(ordinary);
    }
    #[test]
    fn resource_scope_native_root_cannot_be_extended_by_safe_custom_scope() {
        assert_eq!(borrow_token().unwrap_err(), ScopeError::Stale);
        let outer = unsafe { ResourceScope::native_root(Rc::new(Codec(13))) };
        let outer_token = borrow_token().unwrap();
        let inner = unsafe { ResourceScope::native_root(Rc::new(Codec(14))) };
        let inner_token = borrow_token().unwrap();
        let retained = ResourceScope::new(Rc::new(Codec(15)));
        assert_eq!(borrow_token(), Ok(inner_token));
        drop(inner);
        assert_eq!(inner_token.check(), Err(ScopeError::Stale));
        assert_eq!(outer_token.check(), Ok(()));
        assert_eq!(borrow_token(), Ok(outer_token));
        drop(outer);
        assert_eq!(borrow_token().unwrap_err(), ScopeError::Stale);
        assert_eq!(inner_token.check(), Err(ScopeError::Stale));
        drop(retained);
        assert_eq!(borrow_token().unwrap_err(), ScopeError::Stale);
        let safe = ResourceScope::new(Rc::new(Codec(16)));
        assert_eq!(safe.token().check(), Ok(()));
        assert_eq!(borrow_token().unwrap_err(), ScopeError::Stale);
    }
    #[test]
    fn resource_scope_out_of_order_drop_never_revives_expired_codec() {
        let outer = ResourceScope::new(Rc::new(Codec(1)));
        let middle = ResourceScope::new(Rc::new(Codec(2)));
        let inner = ResourceScope::new(Rc::new(Codec(3)));
        assert_eq!(store(Box::new(())), Ok(Id(3)));
        drop(middle);
        assert_eq!(store(Box::new(())), Ok(Id(3)));
        let newest = ResourceScope::new(Rc::new(Codec(4)));
        drop(outer);
        assert_eq!(store(Box::new(())), Ok(Id(4)));
        drop(newest);
        assert_eq!(store(Box::new(())), Ok(Id(3)));
        drop(inner);
        assert_eq!(
            store(Box::new(())),
            Err(WireError::Invalid("no resource context"))
        );
        // A new call after retained guards have gone cannot inherit any expired scope.
        let next = ResourceScope::new(Rc::new(Codec(5)));
        assert_eq!(load::<u64>(Id(0)), Ok(5));
        drop(next);
        assert_eq!(
            store(Box::new(())),
            Err(WireError::Invalid("no resource context"))
        );
    }
    #[test]
    fn resource_scope_callbacks_and_codec_drop_do_not_hold_tls_borrows() {
        struct Reentrant;
        impl ResourceCodec for Reentrant {
            fn store(&self, _: Box<dyn Any>) -> Result<Id, WireError> {
                let _inner = ResourceScope::new(Rc::new(Codec(9)));
                store(Box::new(()))
            }
            fn load(&self, _: Id, _: TypeId) -> Result<Box<dyn Any>, WireError> {
                unreachable!()
            }
        }
        let reentrant = ResourceScope::new(Rc::new(Reentrant));
        assert_eq!(store(Box::new(())), Ok(Id(9)));
        drop(reentrant);
        struct OnDrop(Rc<std::cell::Cell<u64>>);
        impl ResourceCodec for OnDrop {
            fn store(&self, _: Box<dyn Any>) -> Result<Id, WireError> {
                unreachable!()
            }
            fn load(&self, _: Id, _: TypeId) -> Result<Box<dyn Any>, WireError> {
                unreachable!()
            }
        }
        impl Drop for OnDrop {
            fn drop(&mut self) {
                self.0.set(store(Box::new(())).unwrap().0);
            }
        }
        let seen = Rc::new(std::cell::Cell::new(0));
        let outer = ResourceScope::new(Rc::new(OnDrop(seen.clone())));
        let inner = ResourceScope::new(Rc::new(Codec(10)));
        drop(outer);
        assert_eq!(seen.get(), 10);
        drop(inner);
    }
    #[test]
    fn resource_scope_dropping_outer_preserves_only_live_inner() {
        let outer = ResourceScope::new(Rc::new(Codec(6)));
        let retained = ResourceScope::new(Rc::new(Codec(7)));
        drop(outer);
        assert_eq!(store(Box::new(())), Ok(Id(7)));
        drop(retained);
        assert_eq!(
            store(Box::new(())),
            Err(WireError::Invalid("no resource context"))
        );
    }
}
