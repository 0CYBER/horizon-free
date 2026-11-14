//! The mod's side of the boundary, built for `wasm32` (and for native tests behind the
//! `wasm-guest` feature): the transport to the game, the call arena, the handlers the mod gave
//! the game and the entry points the game calls. The generated half (`Guest`, `hook`) sits
//! beside it. See the boundary's design in `horizon-mod-wasm`.
use crate::wire::{Id, Reader, Slots, Wire, WireError, Writer};
use std::any::Any;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

#[path = "prepared.rs"]
mod prepared;
pub use prepared::PreparedSlots;

mod generated {
    #![allow(clippy::all, unused, dead_code, non_snake_case)]
    use crate::__private::game::{BackendMutRefOf, BackendRefOf, ClientBackendMutRefOf};
    use crate::*;
    include!(concat!(env!("OUT_DIR"), "/wire_guest_generated.rs"));
}
pub use generated::{Guest, hook, hook_name};

unsafe extern "Rust" {
    /// The mod's hooks, defined by `#[mod_api::horizon_mod]` in the mod's crate.
    safe fn __horizon_mod_hooks() -> Box<dyn crate::Hooks>;
}

/// The mod's hooks, made once.
fn hooks() -> &'static dyn crate::Hooks {
    static HOOKS: std::sync::OnceLock<Box<dyn crate::Hooks>> = std::sync::OnceLock::new();
    &**HOOKS.get_or_init(__horizon_mod_hooks)
}

/// What one call of the mod needs: where its borrowed returns live, until it returns.
#[derive(Default)]
struct Keep {
    held: Vec<(*mut u8, unsafe fn(*mut u8))>,
}

impl Keep {
    fn hold<T>(&mut self, value: T) -> *mut T {
        unsafe fn drop_box<T>(pointer: *mut u8) {
            // SAFETY: `pointer` came from `Box::into_raw` of a `Box<T>` in `hold`, once.
            drop(unsafe { Box::from_raw(pointer.cast::<T>()) });
        }
        let pointer = Box::into_raw(Box::new(value));
        self.held.push((pointer.cast(), drop_box::<T>));
        pointer
    }
}

impl Drop for Keep {
    fn drop(&mut self) {
        for (pointer, drop) in self.held.drain(..).rev() {
            // SAFETY: each entry is a live box `hold` made, dropped once here, newest first.
            unsafe { drop(pointer) };
        }
    }
}

/// A request acknowledges one exact decoded ID, never another member's state. This
/// remains correct under reentry and when Runtime is member-local within a group.
#[derive(Default)]
struct StaticStrings {
    entries: Vec<Option<&'static str>>,
    last: Option<u32>,
    vector_charge: usize,
}

// Leave at least seven eighths of the group's memory limit for normal mod work.
// The shared counter charges conservative reservations, not just encoded payload bytes.
fn cache_charge(bytes: usize) -> Option<usize> {
    bytes.checked_add(32)?.checked_mul(8)
}

impl Drop for StaticStrings {
    fn drop(&mut self) {
        group_budget_release(self.vector_charge);
    }
}

impl StaticStrings {
    fn resolve(&mut self, id: usize, text: Option<&str>) -> Result<&'static str, WireError> {
        if id >= crate::wire::MAX_STATIC_STRINGS {
            return Err(WireError::Invalid("static string id outside table"));
        }
        let existing = self.entries.get(id).copied().flatten();
        if let Some(text) = text {
            if let Some(existing) = existing {
                if existing != text {
                    return Err(WireError::Invalid("changed static string definition"));
                }
            } else {
                // Intern only on first sight. This preserves canonical static references and
                // the pre-existing bounded storage; known IDs never enter the mutex/hash path.
                let value = crate::wire::intern(text)?;
                if id >= self.entries.len() {
                    let needed = id + 1;
                    if needed > self.entries.capacity() {
                        let wanted = needed
                            .max(self.entries.capacity() * 2)
                            .min(crate::wire::MAX_STATIC_STRINGS);
                        let Some(charge) =
                            cache_charge(wanted * std::mem::size_of::<Option<&'static str>>())
                        else {
                            return Ok(value);
                        };
                        if !group_budget_try_reserve(charge) {
                            return Ok(value);
                        }
                        let mut replacement = Vec::new();
                        if replacement.try_reserve_exact(wanted).is_err() {
                            group_budget_release(charge);
                            return Ok(value);
                        }
                        let Some(actual) = cache_charge(
                            replacement.capacity() * std::mem::size_of::<Option<&'static str>>(),
                        ) else {
                            group_budget_release(charge);
                            return Ok(value);
                        };
                        if actual > charge && !group_budget_try_reserve(actual - charge) {
                            group_budget_release(charge);
                            return Ok(value);
                        }
                        if actual < charge {
                            group_budget_release(charge - actual);
                        }
                        replacement.extend(self.entries.iter().copied());
                        self.entries = replacement;
                        group_budget_release(self.vector_charge);
                        self.vector_charge = actual;
                    }
                    self.entries.resize(needed, None);
                }
                self.entries[id] = Some(value);
            }
        }
        let value = self
            .entries
            .get(id)
            .copied()
            .flatten()
            .ok_or(WireError::Invalid("unknown static string id"))?;
        self.last = Some(id as u32);
        Ok(value)
    }
}

#[derive(Default)]
struct Runtime {
    /// The input the game is writing for the next call.
    pending: Vec<u8>,
    /// One reply per call depth: the game reads it once the call returns.
    replies: Vec<Writer>,
    keeps: Vec<Keep>,
    /// Buffers for arguments and answers, reused so a steady state allocates nothing.
    spare: Vec<Vec<u8>>,
    strings: StaticStrings,
}

impl Runtime {
    fn recycle(&mut self, buffer: Vec<u8>) {
        // Retain buffers for bounded call nesting, not the history of all calls. Large
        // arguments remain allowed, but their storage must not stay pinned in the pool.
        const MAX_SPARE_BUFFERS: usize = 64;
        const MAX_SPARE_CAPACITY: usize = 1024 * 1024;
        if self.spare.len() < MAX_SPARE_BUFFERS && buffer.capacity() <= MAX_SPARE_CAPACITY {
            self.spare.push(buffer);
        }
    }
}

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::default());
}

type Handlers = RefCell<Slots<Vec<Box<dyn Any>>>>;

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static HANDLERS: Handlers = RefCell::new(Slots::default());
}

/// Words in the group's shared state: the std member exports this array as `horizon_group_state`,
/// zeroed, for every mod of the group to read (one memory, one std, one state). A new word is taken
/// from the end of this list; the array's length is part of the ABI.
const GROUP_WORDS: usize = 32;
#[cfg(any(target_arch = "wasm32", test))]
const WORD_HANDLERS: usize = 0;
const WORD_BUDGET: usize = 1;
/// Non-zero while the group's eviction callbacks run, so an allocation that fails inside one fails.
const WORD_EVICTING: usize = 2;
/// The first of the group's eviction slots: each holds a function-table index (the table is the
/// group's, so an index a member registered is callable by any member), or zero when free.
const WORD_EVICT: usize = 3;
/// How many eviction callbacks a group can register.
pub const EVICTORS: usize = GROUP_WORDS - WORD_EVICT;

/// What the mods of one group may hold at once in caches the host asked them to keep, in bytes.
pub const GROUP_BUDGET_BYTES: usize = 256 << 20;

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    /// The group's shared state, defined by the std member.
    #[link_name = "horizon_group_state"]
    static GROUP_STATE: [std::sync::atomic::AtomicUsize; GROUP_WORDS];
}

#[cfg(not(target_arch = "wasm32"))]
static GROUP_STATE: [std::sync::atomic::AtomicUsize; GROUP_WORDS] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; GROUP_WORDS];

/// Takes `bytes` from the group's budget, or answers `false` and takes nothing when that would pass
/// [`GROUP_BUDGET_BYTES`]. Every member counts against the same budget.
pub fn group_budget_try_reserve(bytes: usize) -> bool {
    reserve(group_state(), bytes, GROUP_BUDGET_BYTES)
}

/// Gives `bytes` back to the group's budget; never below zero.
pub fn group_budget_release(bytes: usize) {
    release_budget(group_state(), bytes)
}

fn group_state() -> &'static [std::sync::atomic::AtomicUsize; GROUP_WORDS] {
    // SAFETY: the std member defines the array; atomics need no exclusive access.
    #[cfg(target_arch = "wasm32")]
    return unsafe { &GROUP_STATE };
    #[cfg(not(target_arch = "wasm32"))]
    &GROUP_STATE
}

fn reserve(
    state: &[std::sync::atomic::AtomicUsize; GROUP_WORDS],
    bytes: usize,
    limit: usize,
) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    state[WORD_BUDGET]
        .try_update(Relaxed, Relaxed, |used| {
            used.checked_add(bytes).filter(|total| *total <= limit)
        })
        .is_ok()
}

fn release_budget(state: &[std::sync::atomic::AtomicUsize; GROUP_WORDS], bytes: usize) {
    use std::sync::atomic::Ordering::Relaxed;
    let _ =
        state[WORD_BUDGET].try_update(Relaxed, Relaxed, |used| Some(used.saturating_sub(bytes)));
}

/// Registers `evict` to run when an allocation of the group fails, to free what it can. It runs
/// with the allocator unavailable: it must free memory and must not allocate. Registering the same
/// function again is a no-op. `false` when the group's [`EVICTORS`] slots are taken.
pub fn register_eviction(evict: fn()) -> bool {
    register(group_state(), evict)
}

fn register(state: &[AtomicUsize; GROUP_WORDS], evict: fn()) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let entry = evict as usize;
    for slot in &state[WORD_EVICT..] {
        match slot.compare_exchange(0, entry, Relaxed, Relaxed) {
            Ok(_) => return true,
            Err(held) if held == entry => return true,
            Err(_) => {}
        }
    }
    false
}

/// Runs every registered callback, unless one is running already. `true` when any ran.
fn evict(state: &[AtomicUsize; GROUP_WORDS]) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    if state[WORD_EVICTING].swap(1, Relaxed) != 0 {
        return false;
    }
    let mut ran = false;
    for slot in &state[WORD_EVICT..] {
        let entry = slot.load(Relaxed);
        if entry != 0 {
            // SAFETY: the value was a `fn()` cast by `register`, a table index the whole group shares.
            let callback: fn() = unsafe { std::mem::transmute::<usize, fn()>(entry) };
            callback();
            ran = true;
        }
    }
    state[WORD_EVICTING].store(0, Relaxed);
    ran
}

// exhaustive: wasm boundary plumbing that generated code builds field by field
/// The group's allocator: `inner` first, and when it answers null, the group's eviction callbacks
/// and `inner` once more. The success path is `inner` and one null check.
// no wire: process-local allocator, not a boundary value.
pub struct EvictingAlloc<A> {
    inner: A,
    state: fn() -> &'static [AtomicUsize; GROUP_WORDS],
}

impl<A> EvictingAlloc<A> {
    /// An allocator over `inner` that evicts the group's caches before it fails.
    pub const fn new(inner: A, state: fn() -> &'static [AtomicUsize; GROUP_WORDS]) -> Self {
        Self { inner, state }
    }

    #[cold]
    fn retry(&self, again: impl FnOnce() -> *mut u8) -> *mut u8 {
        if evict((self.state)()) {
            again()
        } else {
            std::ptr::null_mut()
        }
    }
}

// SAFETY: every method answers what `inner` answers, after callbacks that only free memory.
unsafe impl<A: std::alloc::GlobalAlloc> std::alloc::GlobalAlloc for EvictingAlloc<A> {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let block = unsafe { self.inner.alloc(layout) };
        if !block.is_null() {
            return block;
        }
        self.retry(|| unsafe { self.inner.alloc(layout) })
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        let block = unsafe { self.inner.alloc_zeroed(layout) };
        if !block.is_null() {
            return block;
        }
        self.retry(|| unsafe { self.inner.alloc_zeroed(layout) })
    }
    unsafe fn dealloc(&self, block: *mut u8, layout: std::alloc::Layout) {
        unsafe { self.inner.dealloc(block, layout) }
    }
    unsafe fn realloc(&self, block: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        let moved = unsafe { self.inner.realloc(block, layout, size) };
        if !moved.is_null() {
            return moved;
        }
        self.retry(|| unsafe { self.inner.realloc(block, layout, size) })
    }
}

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOCATOR: EvictingAlloc<std::alloc::System> =
    EvictingAlloc::new(std::alloc::System, group_state);

/// Runs `f` on the handlers the game holds for this mod's group. Members of a group share one
/// memory and one std, so they share one table: a handler a member gave the game is found by
/// another member of its group when the game returns its id. The table is made by the first
/// member to need it and lives as long as the group's memory.
fn with_handlers<R>(f: impl FnOnce(&Handlers) -> R) -> R {
    #[cfg(not(target_arch = "wasm32"))]
    {
        HANDLERS.with(f)
    }
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::atomic::Ordering::Relaxed;
        // The handler word is zero until a member makes the table; a wasm group runs on one thread.
        let word = &group_state()[WORD_HANDLERS];
        let mut table = word.load(Relaxed) as *const Handlers;
        if table.is_null() {
            table = Box::into_raw(Box::new(Handlers::default()));
            word.store(table as usize, Relaxed);
        }
        // SAFETY: made above or by another member of this group from the same type, never freed.
        f(unsafe { &*table })
    }
}

/// One exact static reply ID decoded by this instance, or the reserved no-ID marker.
pub fn static_ack() -> u32 {
    RUNTIME.with(|rt| rt.borrow().strings.last.unwrap_or(u32::MAX))
}

/// Model a fresh wasm instance in a native transport fixture.
#[cfg(not(target_arch = "wasm32"))]
pub fn reset_static_strings() {
    RUNTIME.with(|rt| rt.borrow_mut().strings = StaticStrings::default());
}

/// A buffer to write a call's arguments into.
pub fn begin() -> Writer {
    let mut writer = Writer::new();
    if let Some(buffer) = RUNTIME.with(|rt| rt.borrow_mut().spare.pop()) {
        writer.replace(buffer);
    }
    writer
}

// exhaustive: wasm boundary plumbing that generated code builds field by field
/// The game's answer to a call, read once; its buffer goes back to the pool when it drops.
// no wire: owns a transport acknowledgement, released locally.
pub struct Reply(Vec<u8>);

impl Reply {
    /// Decode a static string table reply and its scalar writebacks atomically.
    pub fn take_static<T: Wire>(&self) -> Result<(&'static str, T), WireError> {
        enum Value<'a> {
            Known(u32),
            Definition(u32, &'a str),
            Uncached(&'a str),
        }
        fn text<'a>(reader: &mut Reader<'a>) -> Result<&'a str, WireError> {
            let len = reader.take_len(1)?;
            std::str::from_utf8(reader.take_bytes(len)?).map_err(|_| WireError::Utf8)
        }
        let mut reader = Reader::new(&self.0);
        let tag: u8 = reader.take()?;
        let id: u32 = reader.take()?;
        let value = match tag {
            0 => Value::Known(id),
            1 => Value::Definition(id, text(&mut reader)?),
            2 if id == u32::MAX => Value::Uncached(text(&mut reader)?),
            tag => return Err(WireError::Tag(u32::from(tag))),
        };
        let tail = reader.take()?;
        if !reader.rest().is_empty() {
            return Err(WireError::Trailing);
        }
        let string = match value {
            Value::Known(id) => {
                RUNTIME.with(|rt| rt.borrow_mut().strings.resolve(id as usize, None))?
            }
            Value::Definition(id, text) => {
                RUNTIME.with(|rt| rt.borrow_mut().strings.resolve(id as usize, Some(text)))?
            }
            Value::Uncached(text) => crate::wire::intern(text)?,
        };
        Ok((string, tail))
    }

    /// The value of type `T` the game answered with.
    pub fn take<T: Wire>(&self) -> Result<T, WireError> {
        let mut reader = Reader::new(&self.0);
        let value = reader.take()?;
        match reader.rest().is_empty() {
            true => Ok(value),
            false => Err(WireError::Trailing),
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        let buffer = std::mem::take(&mut self.0);
        RUNTIME.with(|rt| rt.borrow_mut().recycle(buffer));
    }
}

/// Calls the game's import `id` with `args` and returns its answer.
pub fn host(id: u32, args: Writer) -> Reply {
    let mut out = RUNTIME
        .with(|rt| rt.borrow_mut().spare.pop())
        .unwrap_or_default();
    out.clear();
    out.reserve(256);
    transport::call(id, args.bytes(), &mut out);
    RUNTIME.with(|rt| rt.borrow_mut().recycle(args.into_bytes()));
    Reply(out)
}

#[cfg(target_arch = "wasm32")]
mod transport {
    #[link(wasm_import_module = "horizon")]
    unsafe extern "C" {
        fn host(id: u32, in_ptr: *const u8, in_len: u32, out_ptr: *mut u8, out_cap: u32) -> u32;
        fn host_reply(out_ptr: *mut u8);
    }

    pub(super) fn try_call(id: u32, input: &[u8], out: &mut Vec<u8>) -> bool {
        call(id, input, out);
        true
    }
    /// Sends a panic's text to the game: every panic in this module is the guest's own.
    pub(super) fn try_log(input: &[u8]) -> bool {
        try_call(crate::wire::HOST_LOG, input, &mut Vec::new())
    }
    pub(super) fn call(id: u32, input: &[u8], out: &mut Vec<u8>) {
        let cap = out.capacity();
        // SAFETY: the game writes at most `cap` bytes at `out`'s spare capacity, which it owns;
        // a longer answer is kept by the game and copied by `host_reply` into the grown buffer.
        unsafe {
            let len = host(
                id,
                input.as_ptr(),
                input.len() as u32,
                out.as_mut_ptr(),
                cap as u32,
            ) as usize;
            if len > cap {
                out.reserve(len);
                host_reply(out.as_mut_ptr());
            }
            out.set_len(len);
        }
    }
}

/// On a native build the game's side is the test's: set it with [`set_transport`].
#[cfg(not(target_arch = "wasm32"))]
mod transport {
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Serves import `id` for `input`, returning its answer.
    // no wire: process-local boundary service, not a value.
    pub type Transport = Rc<dyn Fn(u32, &[u8]) -> Vec<u8>>;

    thread_local!(static TRANSPORT: RefCell<Option<Transport>> = const { RefCell::new(None) });

    pub fn set(transport: Option<Transport>) {
        TRANSPORT.with(|t| *t.borrow_mut() = transport);
    }

    thread_local!(static SERVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });

    /// Clears [`SERVING`] when the game's answer returns or a panic in it unwinds.
    struct Serving(bool);
    impl Drop for Serving {
        fn drop(&mut self) {
            let _ = SERVING.try_with(|serving| serving.set(self.0));
        }
    }

    pub(super) fn try_call(id: u32, input: &[u8], out: &mut Vec<u8>) -> bool {
        let transport = TRANSPORT
            .try_with(|t| t.try_borrow().ok().and_then(|t| t.clone()))
            .ok()
            .flatten();
        let Some(transport) = transport else {
            return false;
        };
        let _serving = Serving(SERVING.with(|serving| serving.replace(true)));
        out.extend_from_slice(&transport(id, input));
        true
    }
    /// Sends a panic's text to the game, unless the panic is raised while the game answers this
    /// thread, the game's own or a mod hook's re-entered from inside that answer: the game is
    /// mid-call there, so it is printed instead (native harness only).
    pub(super) fn try_log(input: &[u8]) -> bool {
        !SERVING.try_with(std::cell::Cell::get).unwrap_or(true)
            && try_call(crate::wire::HOST_LOG, input, &mut Vec::new())
    }
    pub(super) fn call(id: u32, input: &[u8], out: &mut Vec<u8>) {
        assert!(try_call(id, input, out), "no transport to the game is set");
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use transport::{Transport, set as set_transport};

/// `value`, kept until the call in progress returns, for a method that answers with a borrow.
///
/// # Safety
/// The result may be used only while that call runs: the one lifetime `'a` the caller is
/// given by its own `&self`, which is local to the hook's dispatch.
pub unsafe fn kept<'a, T>(value: T) -> &'a T {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        let keep = match rt.keeps.last_mut() {
            Some(keep) => keep,
            None => {
                rt.keeps.push(Keep::default());
                rt.keeps.last_mut().expect("just pushed")
            }
        };
        // SAFETY: the box stays allocated and unmoved until the call's `Keep` drops.
        unsafe { &*keep.hold(value) }
    })
}

/// Keeps the mod's handler `views` for the game's calls and gives its id.
pub fn add_handler(views: Vec<Box<dyn Any>>) -> Id {
    with_handlers(|h| h.borrow_mut().insert_long(views))
}

/// One view of a handler: it as the trait `T`.
struct View<T: ?Sized + 'static>(Arc<T>);

/// `handler` as a view the game's calls through `T` find.
pub fn view<T: ?Sized + 'static>(handler: Arc<T>) -> Box<dyn Any> {
    Box::new(View::<T>(handler))
}

/// The handler `id` as `T`: stale when the game let it go or never had it from this mod.
pub fn handler<T: ?Sized + 'static>(id: Id) -> Result<Arc<T>, WireError> {
    with_handlers(|h| {
        let handlers = h.borrow();
        let views = handlers
            .get(id)
            .map_err(|_| WireError::Invalid("a handler the game no longer holds"))?;
        views
            .iter()
            .find_map(|v| v.downcast_ref::<View<T>>())
            .map(|v| v.0.clone())
            .ok_or(WireError::Invalid("a handler of another kind"))
    })
}

/// A call the boundary does not carry yet: it traps with what was called.
pub fn unsupported(what: &str) -> ! {
    panic!("{what} is not available to wasm mods yet")
}

// exhaustive: wasm boundary plumbing that generated code builds field by field
/// The game's object the mod holds by `id`: each of its methods is a call to the game, and it
/// is released when the mod drops it.
#[derive(Debug)]
// no wire: owns a live resource-table lease.
pub struct Held(pub Id);

impl Drop for Held {
    fn drop(&mut self) {
        let mut args = begin();
        args.put(&self.0);
        host(crate::wire::HOST_RELEASE, args);
    }
}

/// The game's buffer for the call about to run, `len` bytes.
pub fn input(len: usize) -> *mut u8 {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        if rt.pending.capacity() == 0 {
            rt.pending = rt.spare.pop().unwrap_or_default();
        }
        rt.pending.clear();
        rt.pending.resize(len, 0);
        rt.pending.as_mut_ptr()
    })
}

/// A generated export table sharing the hook transport and call arena.
// no wire: borrowed extension call context.
pub type Extension = fn(u32, &mut Reader<'_>, &mut Writer) -> Result<(), WireError>;

thread_local! {
    static EXTENSIONS: RefCell<Vec<(u32, u32, Extension)>> = const { RefCell::new(Vec::new()) };
}

/// Installs a generated export table for ids matching `(id & mask) == base`.
/// Called while the guest loads, before any of its listeners run.
pub fn extend(base: u32, mask: u32, dispatch: Extension) {
    EXTENSIONS.with(|tables| {
        let mut tables = tables.borrow_mut();
        if let Some(table) = tables.iter_mut().find(|(b, m, _)| *b == base && *m == mask) {
            table.2 = dispatch;
        } else {
            tables.push((base, mask, dispatch));
        }
    });
}

fn dispatch(id: u32, input: &mut Reader<'_>, out: &mut Writer) -> Result<(), WireError> {
    let extension = EXTENSIONS.with(|tables| {
        tables
            .borrow()
            .iter()
            .find(|(base, mask, _)| id & mask == *base)
            .map(|(_, _, f)| *f)
    });
    match extension {
        Some(call) => call(id, input, out),
        None => hook(id, input, out),
    }
}

// hidden: wasm boundary plumbing used by generated code, not a mod's API
/// Installs the guest's resource registry for a synchronous wire operation.
#[doc(hidden)]
pub fn resource_scope() -> crate::wire::ResourceScope {
    generated::resource_scope()
}

/// Runs export `id` on the input the game wrote and returns where its reply is: the pointer in
/// the high half, the length in the low. A refused call answers `u32::MAX` as the length and
/// its reason, as a string, at the pointer.
pub fn run(id: u32) -> (*const u8, u32) {
    static HOOK: std::sync::Once = std::sync::Once::new();
    // A trap says nothing about why: the game gets the panic's text first.
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // A panic can originate while the pool is borrowed, outside a guest call in a
            // native harness, or in a native harness's game while it answers. Logging must not
            // borrow the pool, require a transport or re-enter the game's answer.
            let mut args = Writer::new();
            args.put_str(&info.to_string());
            if !transport::try_log(args.bytes()) {
                previous(info);
            }
        }));
    });
    let _resources = resource_scope();
    let _ = hooks();
    let (input, depth, mut reply) = RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        let depth = rt.keeps.len();
        rt.keeps.push(Keep::default());
        if rt.replies.len() <= depth {
            rt.replies.push(Writer::new());
        }
        let mut reply = std::mem::take(&mut rt.replies[depth]);
        reply.clear();
        (std::mem::take(&mut rt.pending), depth, reply)
    });
    let mut reader = Reader::new(&input);
    let result = match id {
        crate::wire::INIT => reader
            .take::<Vec<crate::PackageInfo>>()
            .map(crate::install_guest_packages),
        crate::wire::RELEASE => reader.take::<Vec<Id>>().map(release),
        id => dispatch(id, &mut reader, &mut reply),
    };
    let result = result.and_then(|()| reply.error().map_or(Ok(()), Err));
    let result = result.and_then(|()| match reader.rest().is_empty() {
        true => Ok(()),
        false => Err(WireError::Trailing),
    });
    let outcome = match result {
        Ok(()) => reply.bytes().len() as u32,
        Err(error) => {
            reply.clear();
            reply.put_str(&error.to_string());
            u32::MAX
        }
    };
    let pointer = reply.bytes().as_ptr();
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        // The borrows of this call end with it, newest first, outside the runtime's borrow.
        let keep = rt.keeps.pop();
        rt.replies[depth] = reply;
        rt.recycle(input);
        drop(rt);
        drop(keep);
    });
    (pointer, outcome)
}

/// Drops the handlers the game let go of. A handler's drop may call the game: not under the lock.
fn release(ids: Vec<Id>) {
    for id in ids {
        let views = with_handlers(|h| h.borrow_mut().remove(id));
        drop(views);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn attachment_refusals_decode_in_guest_reply_without_reinterning() {
        crate::wire::initialize().unwrap();
        let mut writer = Writer::new();
        for value in [
            crate::AttachmentRowError::HolderBusy,
            crate::AttachmentRowError::UnsupportedHolder("dragonpart"),
        ] {
            writer.clear();
            writer.put(&value);
            assert_eq!(Reply(writer.bytes().to_vec()).take(), Ok(value));
        }
        let bound: &'static str = Box::leak("é".repeat(128).into_boxed_str());
        writer.clear();
        writer.put(&crate::AttachmentRowError::UnsupportedHolder(bound));
        assert_eq!(
            Reply(writer.bytes().to_vec()).take(),
            Ok(crate::AttachmentRowError::UnsupportedHolder(bound))
        );
        writer.clear();
        writer.put(&crate::AttachmentError::UnsupportedHolder(bound));
        assert_eq!(
            Reply(writer.bytes().to_vec()).take(),
            Ok(crate::AttachmentError::UnsupportedHolder(bound))
        );
        writer.clear();
        writer.put(&3u32);
        writer.put(&"é".repeat(129));
        assert_eq!(
            Reply(writer.bytes().to_vec()).take::<crate::AttachmentRowError>(),
            Err(WireError::Invalid("static string too long"))
        );
        writer.clear();
        writer.put(&crate::AttachmentError::UnsupportedHolder("dragon"));
        let reply = Reply(writer.bytes().to_vec());
        let crate::AttachmentError::UnsupportedHolder(first) = reply.take().unwrap() else {
            unreachable!()
        };
        for _ in 0..32 {
            let crate::AttachmentError::UnsupportedHolder(next) = reply.take().unwrap() else {
                unreachable!()
            };
            assert!(std::ptr::eq(first.as_ptr(), next.as_ptr()));
        }
        writer.clear();
        writer.put(&crate::AttachmentError::HolderBusy);
        assert_eq!(
            Reply(writer.bytes().to_vec()).take::<crate::AttachmentError>(),
            Ok(crate::AttachmentError::HolderBusy)
        );
        writer.clear();
        writer.put(&8u32);
        writer.put(&"x".repeat(257));
        assert_eq!(
            Reply(writer.bytes().to_vec()).take::<crate::AttachmentError>(),
            Err(WireError::Invalid("static string too long"))
        );
    }

    trait Probe {
        fn answer(&self) -> u32;
    }
    struct Seven;
    impl Probe for Seven {
        fn answer(&self) -> u32 {
            7
        }
    }

    /// A heap of `CAP` bytes over the system's, as the group's memory is.
    struct Limited {
        used: std::sync::atomic::AtomicUsize,
    }
    const CAP: usize = 1000;
    static HEAP: Limited = Limited {
        used: AtomicUsize::new(0),
    };
    unsafe impl std::alloc::GlobalAlloc for Limited {
        unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
            use std::sync::atomic::Ordering::Relaxed;
            let taken = self.used.try_update(Relaxed, Relaxed, |used| {
                (used + layout.size() <= CAP).then_some(used + layout.size())
            });
            if taken.is_err() {
                return std::ptr::null_mut();
            }
            unsafe { std::alloc::System.alloc(layout) }
        }
        unsafe fn dealloc(&self, block: *mut u8, layout: std::alloc::Layout) {
            self.used
                .fetch_sub(layout.size(), std::sync::atomic::Ordering::Relaxed);
            unsafe { std::alloc::System.dealloc(block, layout) }
        }
    }

    /// The shared `HEAP`, as a value the allocator can own.
    struct Heap;
    unsafe impl std::alloc::GlobalAlloc for Heap {
        unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
            unsafe { HEAP.alloc(layout) }
        }
        unsafe fn dealloc(&self, block: *mut u8, layout: std::alloc::Layout) {
            unsafe { HEAP.dealloc(block, layout) }
        }
    }

    static EVICT_STATE: [AtomicUsize; GROUP_WORDS] = [const { AtomicUsize::new(0) }; GROUP_WORDS];
    fn evict_state() -> &'static [AtomicUsize; GROUP_WORDS] {
        &EVICT_STATE
    }
    /// What a cache holds: a block it frees when asked, and how often it was asked.
    static CACHED: std::sync::Mutex<Option<usize>> = std::sync::Mutex::new(None);
    static ASKED: std::sync::atomic::AtomicUsize = AtomicUsize::new(0);
    const BLOCK: std::alloc::Layout = match std::alloc::Layout::from_size_align(800, 1) {
        Ok(layout) => layout,
        Err(_) => panic!(),
    };
    fn free_the_cache() {
        ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(block) = CACHED.lock().unwrap().take() {
            unsafe { std::alloc::GlobalAlloc::dealloc(&HEAP, block as *mut u8, BLOCK) };
        }
    }
    fn keep_the_cache() {
        ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn an_allocation_that_fails_succeeds_once_eviction_frees_memory_and_fails_when_it_frees_nothing()
     {
        use std::alloc::{GlobalAlloc, Layout};
        let alloc = EvictingAlloc::new(Heap, evict_state);
        let want = Layout::from_size_align(500, 1).unwrap();
        // Nothing registered: a failing allocation fails and runs nothing.
        let cache = unsafe { HEAP.alloc(BLOCK) };
        assert!(!cache.is_null());
        assert!(unsafe { alloc.alloc(want) }.is_null());
        assert_eq!(ASKED.load(std::sync::atomic::Ordering::Relaxed), 0);
        // Evicting nothing: asked once, still fails.
        assert!(register(&EVICT_STATE, keep_the_cache));
        assert!(
            register(&EVICT_STATE, keep_the_cache),
            "registering again is idempotent"
        );
        assert!(unsafe { alloc.alloc(want) }.is_null());
        assert_eq!(ASKED.load(std::sync::atomic::Ordering::Relaxed), 1);
        // Evicting the cache: the retry succeeds, and a success never asks.
        *CACHED.lock().unwrap() = Some(cache as usize);
        assert!(register(&EVICT_STATE, free_the_cache));
        let block = unsafe { alloc.alloc(want) };
        assert!(!block.is_null());
        assert_eq!(ASKED.load(std::sync::atomic::Ordering::Relaxed), 3);
        let small = unsafe { alloc.alloc(Layout::from_size_align(8, 1).unwrap()) };
        assert!(!small.is_null());
        assert_eq!(ASKED.load(std::sync::atomic::Ordering::Relaxed), 3);
        unsafe {
            alloc.dealloc(block, want);
            alloc.dealloc(small, Layout::from_size_align(8, 1).unwrap());
        }
    }

    #[test]
    fn eviction_slots_are_bounded_and_one_callback_runs_at_a_time() {
        let state = [const { AtomicUsize::new(0) }; GROUP_WORDS];
        fn a() {}
        fn b() {}
        assert!(register(&state, a));
        for slot in &state[WORD_EVICT + 1..] {
            slot.store(usize::MAX, std::sync::atomic::Ordering::Relaxed);
        }
        assert!(register(&state, a), "an entry already held registers again");
        assert!(!register(&state, b), "no slot is left");
        // A callback that runs inside another's run is refused.
        state[WORD_EVICTING].store(1, std::sync::atomic::Ordering::Relaxed);
        assert!(!evict(&state));
    }

    #[test]
    fn group_budget_is_one_counter_for_every_member_and_never_overdraws() {
        use std::sync::atomic::AtomicUsize;
        let state = [const { AtomicUsize::new(0) }; GROUP_WORDS];
        // Two members reach the same words, so their reservations add up.
        assert!(reserve(&state, 60, 100));
        assert!(reserve(&state, 40, 100));
        assert!(!reserve(&state, 1, 100), "the limit is exact");
        assert!(!reserve(&state, usize::MAX, 100), "overflow takes nothing");
        release_budget(&state, 40);
        assert!(
            reserve(&state, 40, 100),
            "released bytes are available again"
        );
        release_budget(&state, 1000);
        assert_eq!(
            state[WORD_BUDGET].load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            state[WORD_HANDLERS].load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert!(group_budget_try_reserve(1));
        group_budget_release(1);
    }

    #[test]
    fn handlers_resolve_until_released_and_forged_ids_are_refused() {
        let provider: Arc<dyn Probe> = Arc::new(Seven);
        let id = add_handler(vec![view(provider)]);
        assert_eq!(handler::<dyn Probe>(id).unwrap().answer(), 7);
        // Another kind, an unissued slot and a stale generation answer with errors, never a panic.
        assert!(matches!(handler::<dyn Any>(id), Err(WireError::Invalid(_))));
        assert!(handler::<dyn Probe>(Id((1 << 32) | 0xffff)).is_err());
        assert!(handler::<dyn Probe>(Id(0)).is_err());
        release(vec![id]);
        assert!(matches!(
            handler::<dyn Probe>(id),
            Err(WireError::Invalid("a handler the game no longer holds"))
        ));
    }

    #[test]
    fn guest_pool_bounds_returned_leases_and_discards_large_buffers() {
        let mut runtime = Runtime::default();
        let leases: Vec<_> = (0..100).map(|_| Vec::with_capacity(32)).collect();
        for buffer in leases {
            runtime.recycle(buffer);
        }
        assert_eq!(runtime.spare.len(), 64);
        let mut runtime = Runtime::default();
        runtime.recycle(Vec::with_capacity(1024 * 1024 + 1));
        assert!(runtime.spare.is_empty());
    }

    #[test]
    fn guest_pool_returns_nested_import_arguments_and_replies() {
        set_transport(Some(std::rc::Rc::new(|id, bytes| {
            if id == 0 {
                assert_eq!(Reader::decode::<u32>(bytes).unwrap(), 5);
                let mut arguments = begin();
                arguments.put(&37u32);
                assert_eq!(host(1, arguments).take::<u32>().unwrap(), 99);
            } else {
                assert_eq!(Reader::decode::<u32>(bytes).unwrap(), 37);
            }
            99u32.to_le_bytes().to_vec()
        })));
        let mut arguments = begin();
        arguments.put(&5u32);
        let reply = host(0, arguments);
        assert_eq!(reply.take::<u32>().unwrap(), 99);
        // The nested arguments and reply, plus outer arguments, have returned. The
        // outer answer still belongs to Reply and cannot be leased by another call.
        RUNTIME.with(|rt| assert_eq!(rt.borrow().spare.len(), 3));
        drop(reply);
        RUNTIME.with(|rt| assert_eq!(rt.borrow().spare.len(), 4));
        set_transport(None);
    }

    #[test]
    fn guest_pool_does_not_cache_large_import_arguments_or_answers() {
        set_transport(Some(std::rc::Rc::new(|_, bytes| {
            assert!(bytes.len() > 1024 * 1024);
            vec![7]
        })));
        let mut arguments = begin();
        arguments.put_str(&"x".repeat(1024 * 1024 + 1));
        let reply = host(0, arguments);
        assert_eq!(reply.0, [7]);
        RUNTIME.with(|rt| assert!(rt.borrow().spare.is_empty()));
        drop(reply);
        RUNTIME.with(|rt| assert_eq!(rt.borrow().spare.len(), 1));
        set_transport(Some(std::rc::Rc::new(|_, _| vec![7; 1024 * 1024 + 1])));
        let mut arguments = begin();
        arguments.put(&5u32);
        let reply = host(0, arguments);
        assert_eq!(reply.0.len(), 1024 * 1024 + 1);
        drop(reply);
        RUNTIME.with(|rt| {
            let runtime = rt.borrow();
            assert_eq!(runtime.spare.len(), 1);
            assert!(runtime.spare[0].capacity() <= 1024 * 1024);
        });
        set_transport(None);
    }
}

#[cfg(target_arch = "wasm32")]
mod entry {
    use super::*;

    /// The layout of every value that crosses, which the game compares with its own.
    #[unsafe(no_mangle)]
    pub extern "C" fn horizon_layout() -> u64 {
        *crate::wire::LAYOUT
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn horizon_in(len: u32) -> *mut u8 {
        input(len as usize)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn horizon_hook(id: u32) -> u64 {
        let (pointer, len) = run(id);
        (pointer as usize as u64) << 32 | u64::from(len)
    }
}
