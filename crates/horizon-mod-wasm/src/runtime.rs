//! Sandboxed core-wasm transport. One lock, memory and poison flag per dependency group.
//! Ordinary refusals leave the group running. Faults, native unwinds and a failed
//! prepared release poison it, since the guest may still own retained state.
//! The preallocated call path adds an Arc increment/decrement per call, without
//! allocating on success. Diagnostic keys are built and checked only on refusal.
#[cfg(all(test, feature = "test-copy-probe"))]
#[path = "equip_scope_tests.rs"]
mod equip_scope_tests;
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;
#[cfg(test)]
#[path = "threads_tests.rs"]
mod threads_tests;
use crate::{
    Refusal,
    boundary::{Cx, Frame, serve},
    link::{Group, Member},
};
use mod_api::{
    HookId,
    wire::{Id, Reader, WireError, Writer},
};
use std::{
    cell::Cell,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use wasmtime::{
    AsContextMut, Caller, Engine, Extern, Func, Memory, Store, StoreContextMut, TypedFunc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallClass {
    Load,
    Hook,
    Worker,
    Callback,
}

/// A group's linear memory when none of its mods declares one (`wasm_memory_mib`): 1 GiB.
pub(crate) const DEFAULT_GROUP_MEMORY_MIB: u32 = 1024;
/// The function table's ceiling: references, a pointer each.
const TABLE_ELEMENTS: usize = 1 << 20;

/// A group's ceilings. The memory limit is a ceiling, not committed memory: past it, or past what
/// the system gives, `memory.grow` answers -1 inside the guest and the call goes on.
pub(crate) struct Limits {
    /// The group's mods, for the log.
    group: Arc<str>,
    memory: usize,
    /// The size the last allowed `memory.grow` asked for, until the system grants or refuses it.
    allowed_growth: usize,
    /// The size the last refused `memory.grow` asked for, and whether the system (not the limit)
    /// refused it, for the reason of a call that then failed.
    refused_growth: Option<(usize, bool)>,
    /// Whether the log already named this group's limit, so a guest at its limit logs once.
    warned: bool,
}
impl Limits {
    pub(crate) fn new(group: Arc<str>, memory_mib: u32) -> Self {
        Self {
            group,
            memory: usize::try_from(u64::from(memory_mib) << 20).unwrap_or(usize::MAX),
            allowed_growth: 0,
            refused_growth: None,
            warned: false,
        }
    }
}
impl wasmtime::ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired <= self.memory && maximum.is_none_or(|maximum| desired <= maximum) {
            self.allowed_growth = desired;
            return Ok(true);
        }
        self.refused_growth = Some((desired, false));
        if !std::mem::replace(&mut self.warned, true) {
            log::warn!(
                "wasm group {}: memory.grow to {} MiB refused at the group's limit of {} MiB",
                self.group,
                desired >> 20,
                self.memory >> 20
            );
        }
        Ok(false)
    }
    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.refused_growth = Some((self.allowed_growth, true));
        log::warn!(
            "wasm group {}: the system refused memory.grow below the group's limit of {} MiB: {error:#}",
            self.group,
            self.memory >> 20
        );
        Ok(())
    }
    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= TABLE_ELEMENTS && maximum.is_none_or(|maximum| desired <= maximum))
    }
}

pub(crate) struct State {
    limits: Limits,
    cx: Box<Cx>,
    memory: Option<Memory>,
    replies: Vec<Writer>,
    inputs: Vec<Writer>,
    outputs: Vec<Vec<u8>>,
    imports: Vec<Vec<u8>>,
    depth: usize,
    path: Vec<(Arc<str>, u32)>,
    refusals: std::collections::HashSet<String>,
    epoch_budget: u64,
    group: usize,
    lending: Arc<AtomicUsize>,
}
// SAFETY: frames contain borrowed native objects only while the group mutex is held.
// They are pushed and popped synchronously on that thread; callbacks borrow the same
// locked store through a thread-local loan. The module pointer borrows call()'s self
// under the same scope. Between threads the frame stack is empty and module is null.
unsafe impl Send for State {}
impl State {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            cx: Box::default(),
            memory: None,
            replies: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            imports: Vec::new(),
            depth: 0,
            path: Vec::with_capacity(64),
            refusals: Default::default(),
            epoch_budget: 0,
            group: 0,
            lending: Arc::default(),
        }
    }
}

/// Explicit engine settings; the remaining defaults are covered by Wasmtime compatibility.
pub(crate) const ENGINE_CONFIG: (&str, bool) = ("epoch_interruption", true);

/// Wasm threads stay off: Wasmtime's `threads` feature is compiled in, which turns the proposal on
/// by default, but no mod may define or import shared memory or run an atomic instruction. Pulley
/// has no shared memory either way.
pub(crate) const THREADS: [(&str, bool); 2] = [("wasm_threads", false), ("shared_memory", false)];

/// Every engine that compiles or runs mod code, tests included, takes [`THREADS`] from here.
pub(crate) fn without_threads(config: &mut wasmtime::Config) -> &mut wasmtime::Config {
    config
        .wasm_threads(THREADS[0].1)
        .shared_memory(THREADS[1].1)
}

/// Wasmtime's other defaults with [`THREADS`]: for tests that need no epoch deadlines.
#[cfg(test)]
pub(crate) fn test_engine() -> Engine {
    Engine::new(without_threads(&mut wasmtime::Config::new())).unwrap()
}

/// The settings of the engine mods run on, without its epoch ticker.
pub(crate) fn engine_config() -> wasmtime::Config {
    let mut config = wasmtime::Config::new();
    without_threads(&mut config);
    config.epoch_interruption(ENGINE_CONFIG.1);
    config.max_wasm_stack(horizon_guest_stack::GUEST_STACK_BYTES);
    // Wasmtime validates this bound even for synchronous engines. No async
    // support or fiber execution is enabled by setting its size.
    config.async_stack_size(
        horizon_guest_stack::GUEST_STACK_BYTES + horizon_guest_stack::HOST_HEADROOM,
    );
    config
}

pub(crate) fn engine() -> wasmtime::Result<Engine> {
    let config = engine_config();
    let engine = Engine::new(&config)?;
    let weak = engine.weak();
    std::thread::Builder::new()
        .name("wasm epoch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(1));
                let Some(engine) = weak.upgrade() else { break };
                engine.increment_epoch();
            }
        })?;
    Ok(engine)
}

struct Releases<T> {
    pending: AtomicBool,
    entries: Mutex<Vec<T>>,
}
impl<T> Default for Releases<T> {
    fn default() -> Self {
        Self {
            pending: AtomicBool::new(false),
            entries: Mutex::new(Vec::new()),
        }
    }
}
impl<T> Releases<T> {
    fn push(&self, entry: T) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.push(entry);
        self.pending.store(true, Ordering::Release);
    }
    fn take(&self) -> Vec<T> {
        // A plain load first: the uncontended call path stays read-only.
        if !self.pending.load(Ordering::Acquire) || !self.pending.swap(false, Ordering::AcqRel) {
            return Vec::new();
        }
        // A racing push either joins this drain or leaves pending set for the next
        // call. At worst the next call locks an empty queue; no release is lost.
        std::mem::take(&mut *self.entries.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

struct Busy;

struct Running {
    group: Mutex<Group<State>>,
    poisoned: AtomicBool,
    lending: Arc<AtomicUsize>,
    /// The last top-level call to take `group`: who a refused caller waited on.
    holder: Mutex<Option<Holder>>,
}

/// A top-level call holding a group: its mod, export, thread and when it took the lock.
struct Holder {
    module: Arc<str>,
    export: u32,
    thread: std::thread::Thread,
    since: Instant,
}

impl Running {
    /// Why a caller that waited `waited` for this group is refused, naming the call holding it.
    fn busy(&self, waited: Duration) -> String {
        let holder = self.holder.lock().unwrap_or_else(|e| e.into_inner());
        match holder.as_ref() {
            Some(h) => format!(
                "{}: held by mod {} export {} on thread {} for {} ms; this call waited {} ms",
                Refusal::Busy,
                h.module,
                h.export,
                h.thread.name().map_or_else(
                    || format!("{:?}", h.thread.id()),
                    |name| format!("'{name}'")
                ),
                h.since.elapsed().as_millis(),
                waited.as_millis()
            ),
            None => format!(
                "{}: held outside a guest call; this call waited {} ms",
                Refusal::Busy,
                waited.as_millis()
            ),
        }
    }
}
thread_local! {
    /// Why this thread's last guest call answered nothing, for the caller's typed error.
    static LAST_REFUSAL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}
/// A test's budget for non-load calls in this process, in epoch ticks (about 1 ms each); 0 keeps
/// a hook's 100. Process-wide: the calls it covers run on the game's own threads.
pub(crate) static CALL_BUDGET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn note(reason: String) {
    LAST_REFUSAL.with(|last| *last.borrow_mut() = Some(reason));
}
/// Why this thread's last guest call answered nothing: an epoch deadline, a trap with its
/// code, a refusal by the stack guard, the group's state or the guest's own refusal.
pub(crate) fn take_refusal() -> String {
    LAST_REFUSAL
        .with(|last| last.borrow_mut().take())
        .unwrap_or_else(|| "refused".into())
}
fn fault_reason(error: &wasmtime::Error, budget: u64, limits: &mut Limits) -> String {
    let reason = match error.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => {
            format!(
                "epoch deadline: the call ran past its budget of {budget} ticks (about {budget} ms)"
            )
        }
        Some(trap) => format!("{trap}"),
        None => format!("fault: {error:#}"),
    };
    match limits.refused_growth.take() {
        Some((desired, by_system)) => format!(
            "{reason}, after memory.grow to {} MiB was refused {} group {}'s limit of {} MiB",
            desired >> 20,
            if by_system {
                "by the system below"
            } else {
                "at"
            },
            limits.group,
            limits.memory >> 20
        ),
        None => reason,
    }
}
/// Only faults may leave guest state half-updated. Refusals returned normally.
#[derive(Debug)]
enum CallError {
    Refused(wasmtime::Error),
    Fault(wasmtime::Error),
}
impl CallError {
    fn into_error(self) -> wasmtime::Error {
        match self {
            Self::Refused(error) | Self::Fault(error) => error,
        }
    }
}
impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(error) | Self::Fault(error) => write!(f, "{error:#}"),
        }
    }
}
impl From<WireError> for CallError {
    fn from(error: WireError) -> Self {
        Self::Refused(error.into())
    }
}
struct PoisonOnPanic<'a>(&'a AtomicBool);
impl Drop for PoisonOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.store(true, Ordering::Relaxed);
        }
    }
}
struct FrameScope {
    state: *mut State,
    previous_module: *const WasmMod,
    running: Arc<Running>,
}
impl Drop for FrameScope {
    fn drop(&mut self) {
        // SAFETY: the store is exclusively locked or lent on this thread, and its
        // boxed state remains in place until this synchronous call scope ends.
        let state = unsafe { &mut *self.state };
        state.cx.frames.pop();
        state.path.pop();
        state.cx.module = self.previous_module;
        state.depth -= 1;
        if std::thread::panicking() {
            self.running.poisoned.store(true, Ordering::Relaxed);
        }
        if state.depth == 0 && self.running.poisoned.load(Ordering::Relaxed) {
            // Detach before dropping: native destructors may call a proxy again.
            drop(std::mem::take(&mut state.cx.held));
        }
    }
}
#[derive(Clone)]
pub struct WasmMod {
    id: Arc<str>,
    running: Arc<Running>,
    input: TypedFunc<u32, u32>,
    hook: TypedFunc<u32, u64>,
    pub subscribed: &'static [Option<HookId>],
    pub(crate) listeners: &'static [mod_api::Listener],
    released: Arc<Releases<Id>>,
    prepared_releases: Arc<Releases<(u32, u32)>>,
    prepared: Arc<Mutex<crate::prepared::Slots>>,
    prepared_entries: Arc<std::collections::HashMap<u32, TypedFunc<(), u64>>>,
    subscriptions: Arc<Mutex<std::collections::HashMap<(Id, u32), &'static [Option<HookId>]>>>,
}
/// A guest object never owns the store that may itself retain that object.
#[derive(Clone)]
pub(crate) struct WeakWasmMod {
    id: Arc<str>,
    running: Weak<Running>,
    input: TypedFunc<u32, u32>,
    hook: TypedFunc<u32, u64>,
    subscribed: &'static [Option<HookId>],
    pub(crate) listeners: &'static [mod_api::Listener],
    released: Arc<Releases<Id>>,
    prepared_releases: Arc<Releases<(u32, u32)>>,
    prepared: Arc<Mutex<crate::prepared::Slots>>,
    prepared_entries: Arc<std::collections::HashMap<u32, TypedFunc<(), u64>>>,
    subscriptions: Arc<Mutex<std::collections::HashMap<(Id, u32), &'static [Option<HookId>]>>>,
}
impl WeakWasmMod {
    pub(crate) fn equip_instance(&self) -> usize {
        Arc::as_ptr(&self.prepared) as usize
    }
    /// Whether the module's group has stopped or the module has been dropped.
    pub(crate) fn is_stopped(&self) -> bool {
        self.running
            .upgrade()
            .is_some_and(|running| running.poisoned.load(Ordering::Relaxed))
    }
    pub(crate) fn prepare(&self, bytes: &[u8]) -> Option<(u32, u32)> {
        if self
            .running
            .upgrade()
            .is_none_or(|running| running.poisoned.load(Ordering::Relaxed))
        {
            note("its group has stopped or was unloaded".into());
            return None;
        }
        if self.prepared_entries.len() != 3 {
            note(
                "can_equip requires prepare_can_equip, ask_can_equip and release_can_equip exports"
                    .into(),
            );
            return None;
        }
        let token = self
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepare()?;
        struct Pending<'a> {
            module: &'a WeakWasmMod,
            token: (u32, u32),
            committed: bool,
        }
        impl Drop for Pending<'_> {
            fn drop(&mut self) {
                if !self.committed {
                    self.module.release(self.token.0, self.token.1);
                }
            }
        }
        let mut pending = Pending {
            module: self,
            token,
            committed: false,
        };
        let module = self.upgrade()?;
        let result = module.try_call(
            CallClass::Hook,
            Frame::default(),
            crate::prepared::PREPARE,
            None,
            |w| {
                w.put(&token.0);
                w.put(&token.1);
                w.put_bytes(bytes);
            },
            |r| {
                let echoed: (u32, u32) = r.take()?;
                if echoed != token {
                    return Err(WireError::Invalid("prepared token mismatch"));
                }
                Ok(())
            },
        );
        match result {
            Ok(Some(())) => {}
            Err(Busy) => {
                // The guest never received the preparation.
                let mut slots = self.prepared.lock().unwrap_or_else(|e| e.into_inner());
                slots.release(token.0, token.1);
                slots.finish_release(token.0);
                pending.committed = true;
                return None;
            }
            Ok(None) => return None,
        }
        pending.committed = true;
        Some(token)
    }
    pub(crate) fn check(&self, slot: u32, generation: u32) -> bool {
        self.running
            .upgrade()
            .is_some_and(|r| !r.poisoned.load(Ordering::Relaxed))
            && self
                .prepared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .check(slot, generation)
    }
    pub(crate) fn release(&self, slot: u32, generation: u32) {
        let released = self
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .release(slot, generation);
        if !released {
            return;
        }
        let Some(module) = self.upgrade() else {
            return;
        };
        if std::thread::panicking() {
            module.disable();
            return;
        }
        match module.try_call(
            CallClass::Hook,
            Frame::default(),
            crate::prepared::RELEASE,
            None,
            |w| {
                w.put(&slot);
                w.put(&generation);
            },
            |r| r.take::<()>(),
        ) {
            Ok(Some(())) => module.finish_prepared_release(slot),
            Err(Busy) => self.prepared_releases.push((slot, generation)),
            Ok(None) => module.disable(),
        }
    }
    /// Refusals answer unavailable without stopping the group; faults stop all members.
    pub(crate) fn call<R>(
        &self,
        class: CallClass,
        frame: Frame,
        id: u32,
        handler: Option<Id>,
        args: impl FnOnce(&mut Writer),
        reply: impl FnOnce(&mut Reader<'_>) -> Result<R, WireError>,
    ) -> Option<R> {
        self.upgrade()?.call(class, frame, id, handler, args, reply)
    }

    fn upgrade(&self) -> Option<WasmMod> {
        let Some(running) = self.running.upgrade() else {
            note("the module was dropped".into());
            return None;
        };
        Some(WasmMod {
            id: self.id.clone(),
            running,
            input: self.input.clone(),
            hook: self.hook.clone(),
            subscribed: self.subscribed,
            listeners: self.listeners,
            released: self.released.clone(),
            prepared_releases: self.prepared_releases.clone(),
            prepared: self.prepared.clone(),
            prepared_entries: self.prepared_entries.clone(),
            subscriptions: self.subscriptions.clone(),
        })
    }
}
pub struct WasmHandler {
    pub index: Id,
    pub(crate) module: WeakWasmMod,
}
impl std::fmt::Debug for WasmHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmHandler")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}
impl WasmHandler {
    pub fn new(module: &WasmMod, index: Id) -> Self {
        Self {
            index,
            module: module.downgrade(),
        }
    }
    pub fn belongs_to(&self, module: &WasmMod) -> bool {
        self.module.running.ptr_eq(&Arc::downgrade(&module.running))
    }
    pub fn subscriptions(&self, owner: &'static str, export_id: u32) -> &'static [Option<HookId>] {
        if let Some(value) = self
            .module
            .subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(self.index, export_id))
            .copied()
        {
            return value;
        }
        let Some(names) = self.module.call(
            CallClass::Callback,
            Frame::default(),
            export_id,
            Some(self.index),
            |_| {},
            |r| r.take(),
        ) else {
            return &[];
        };
        let value = crate::boundary::subscriptions_of(owner, names);
        self.module
            .subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((self.index, export_id), value);
        value
    }
}
impl Drop for WasmHandler {
    fn drop(&mut self) {
        self.module
            .subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(id, _), _| *id != self.index);
        self.module.released.push(self.index);
    }
}

// An import lends its context only while native code runs. Removing the pointer while
// guest code executes prevents double borrowing; a nested import installs a fresh node.
thread_local! { static LENT: Cell<*const Loan> = const { Cell::new(std::ptr::null()) }; }
struct Loan {
    group: usize,
    lending: Arc<AtomicUsize>,
    context: Cell<*mut StoreContextMut<'static, State>>,
    previous: *const Loan,
}
impl Drop for Loan {
    fn drop(&mut self) {
        self.lending.fetch_sub(1, Ordering::Relaxed);
        LENT.set(self.previous);
    }
}
struct Resumed {
    loan: *const Loan,
    context: *mut StoreContextMut<'static, State>,
}
impl Resumed {
    fn take(group: usize) -> Option<Self> {
        let mut ptr = LENT.get();
        while !ptr.is_null() {
            // SAFETY: nodes live on synchronous import stacks on this thread.
            let loan = unsafe { &*ptr };
            if loan.group == group {
                let context = loan.context.replace(std::ptr::null_mut());
                return (!context.is_null()).then_some(Self { loan: ptr, context });
            }
            ptr = loan.previous;
        }
        None
    }
    fn context(&mut self) -> StoreContextMut<'_, State> {
        // SAFETY: this loan removed the context exclusively until Drop restores it.
        unsafe { &mut *self.context }.as_context_mut()
    }
}
impl Drop for Resumed {
    fn drop(&mut self) {
        unsafe { &*self.loan }.context.set(self.context);
    }
}

fn host(
    mut caller: Caller<'_, State>,
    id: u32,
    ptr: u32,
    len: u32,
    out: u32,
    cap: u32,
) -> wasmtime::Result<u32> {
    let memory = caller
        .data()
        .memory
        .ok_or_else(|| wasmtime::Error::msg("host import during instantiation"))?;
    let end = (ptr as usize)
        .checked_add(len as usize)
        .ok_or_else(|| wasmtime::Error::msg("input range overflow"))?;
    let depth = caller.data().depth;
    if depth == 0 {
        return Err(wasmtime::Error::msg("host import outside a call"));
    }
    let mut input = std::mem::take(&mut caller.data_mut().imports[depth]);
    input.clear();
    input.extend_from_slice(
        memory
            .data(&caller)
            .get(ptr as usize..end)
            .ok_or_else(|| wasmtime::Error::msg("input outside memory"))?,
    );
    #[cfg(feature = "test-copy-probe")]
    crate::copy_probe::record(false, input.len());
    let mut reply = std::mem::take(&mut caller.data_mut().replies[depth]);
    reply.clear();
    let cx = &mut *caller.data_mut().cx as *mut Cx;
    // SAFETY: this import holds the locked/lent store; Cx stays boxed until the
    // synchronous native dispatch and any nested callbacks have returned.
    let resources = unsafe { crate::boundary::resource_scope(cx, false) };
    let mut context = caller.as_context_mut();
    let lending = context.data().lending.clone();
    lending.fetch_add(1, Ordering::Relaxed);
    let loan = Loan {
        lending,
        group: context.data().group,
        context: Cell::new((&mut context as *mut StoreContextMut<'_, State>).cast()),
        previous: LENT.get(),
    };
    LENT.set(&loan);
    let result = if id == mod_api::wire::HOST_RELEASE {
        let mut reader = Reader::new(&input);
        let id = reader.take::<Id>()?;
        if !reader.rest().is_empty() {
            return Err(WireError::Trailing.into());
        }
        unsafe { &mut *cx }
            .held
            .remove(id)
            .map(|_| ())
            .map_err(|e| wasmtime::Error::msg(format!("{e:?}")))
    } else if id == mod_api::wire::HOST_LOG {
        let mut reader = Reader::new(&input);
        let message: String = reader.take()?;
        if !reader.rest().is_empty() {
            return Err(WireError::Trailing.into());
        }
        // Release the Cx borrow before entering a possibly reentrant logger.
        let module = unsafe { (&*cx).module.as_ref().map(|module| module.id.clone()) }
            .ok_or_else(|| wasmtime::Error::msg("no calling module"))?;
        log::error!("wasm mod {module}: {message}");
        Ok(())
    } else {
        // SAFETY: Cx is boxed and stable; serve must release its frame borrows before
        // invoking native code that can synchronously reenter the guest.
        unsafe { serve(id, cx, &input, &mut reply) }
            .map_err(|e| wasmtime::Error::msg(format!("{e:?}")))
    };
    drop(loan);
    drop(resources);
    caller.data_mut().imports[depth] = input;
    result?;
    if let Some(error) = reply.error() {
        return Err(error.into());
    }
    let size = u32::try_from(reply.bytes().len())?;
    if size != 0 && size <= cap {
        memory.write(&mut caller, out as usize, reply.bytes())?;
        #[cfg(feature = "test-copy-probe")]
        crate::copy_probe::record(true, reply.bytes().len());
    }
    caller.data_mut().replies[depth] = reply;
    Ok(size)
}
fn host_reply(mut caller: Caller<'_, State>, ptr: u32) -> wasmtime::Result<()> {
    let state = caller.data();
    let memory = state
        .memory
        .ok_or_else(|| wasmtime::Error::msg("no group memory"))?;
    let depth = state.depth;
    if depth == 0 {
        return Err(wasmtime::Error::msg("host reply outside a call"));
    }
    let reply = std::mem::take(&mut caller.data_mut().replies[depth]);
    let result = memory.write(&mut caller, ptr as usize, reply.bytes());
    caller.data_mut().replies[depth] = reply;
    result?;
    #[cfg(feature = "test-copy-probe")]
    crate::copy_probe::record(true, caller.data().replies[depth].bytes().len());
    Ok(())
}

impl WasmMod {
    pub(crate) fn check_prepared_hooks(&self, hooks: &[Option<HookId>]) -> Result<(), String> {
        for (hook, entries) in crate::boundary::PREPARED_EXPORTS {
            if !hooks.iter().flatten().any(|id| {
                crate::boundary::hook_id(hook).is_some_and(|wanted| wanted as usize == *id as usize)
            }) {
                continue;
            }
            for (name, id) in entries {
                if !self.prepared_entries.contains_key(id) {
                    return Err(format!(
                        "{hook} requires prepared wasm export {name}; no fallback is available"
                    ));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn downgrade(&self) -> WeakWasmMod {
        WeakWasmMod {
            id: self.id.clone(),
            running: Arc::downgrade(&self.running),
            input: self.input.clone(),
            hook: self.hook.clone(),
            subscribed: self.subscribed,
            listeners: self.listeners,
            released: self.released.clone(),
            prepared_releases: self.prepared_releases.clone(),
            prepared: self.prepared.clone(),
            prepared_entries: self.prepared_entries.clone(),
            subscriptions: self.subscriptions.clone(),
        }
    }
    pub(crate) fn mod_id(&self) -> &str {
        &self.id
    }
    pub(crate) fn is_stopped(&self) -> bool {
        self.running.poisoned.load(Ordering::Relaxed)
    }
    pub(crate) fn disable(&self) {
        // Publish stop before destroying native resources. Never wait on a store
        // loaned to native code: the outermost FrameScope clears it on return.
        self.running.poisoned.store(true, Ordering::Relaxed);
        if let Some(mut resumed) = Resumed::take(Arc::as_ptr(&self.running) as usize) {
            let mut store = resumed.context();
            if store.data().depth == 0 {
                drop(std::mem::take(&mut store.data_mut().cx.held));
            }
        } else if let Ok(mut group) = self.running.group.try_lock() {
            drop(std::mem::take(&mut group.store.data_mut().cx.held));
        }
    }
    pub(crate) fn instantiate(
        engine: &Engine,
        memory_mib: u32,
        members: &[Member<'_>],
        cache: Option<&crate::module_cache::ModuleCache>,
    ) -> Result<Vec<Self>, Refusal> {
        horizon_guest_stack::check_current().map_err(|error| Refusal::Runtime(error.into()))?;
        let group: Vec<_> = members
            .iter()
            .filter(|member| member.name != "libstd.so")
            .map(|member| {
                let name = member.name.strip_suffix(".so").unwrap_or(member.name);
                name.strip_prefix("lib").unwrap_or(name)
            })
            .collect();
        let limits = Limits::new(group.join("+").into(), memory_mib);
        let mut store = Store::new(engine, State::new(limits));
        store.limiter(|state| &mut state.limits);
        store.set_epoch_deadline(30_000);
        let mut armed = false;
        let mut group = Group::link_cached_with(store, members, cache, |store, module, name| {
            // Compilation is not guest execution and must not consume its load deadline.
            if !armed {
                store.set_epoch_deadline(30_000);
                armed = true;
            }
            if module != "horizon" {
                return None;
            }
            match name {
                "host" => Some(Extern::Func(Func::wrap(store, host))),
                "host_reply" => Some(Extern::Func(Func::wrap(store, host_reply))),
                _ => None,
            }
        })
        .map_err(Refusal::from_link)?;
        group.store.data_mut().memory = Some(group.memory);
        let mut exports = Vec::new();
        for (name, instance) in group.members.clone() {
            // The std member is a library, not a mod.
            if instance
                .get_func(&mut group.store, "horizon_layout")
                .is_none()
            {
                continue;
            }
            let load = |e: wasmtime::Error| Refusal::Runtime(format!("{e:#}"));
            let layout = instance
                .get_typed_func::<(), u64>(&mut group.store, "horizon_layout")
                .map_err(load)?
                .call(&mut group.store, ())
                .map_err(load)?;
            if layout != *mod_api::wire::LAYOUT {
                if layout == mod_api::wire_core_layout() {
                    return Err(Refusal::Runtime("prepared can_equip ABI layout mismatch: guest carries the old unprepared layout; rebuild the wasm mod".into()));
                }
                return Err(Refusal::Layout {
                    built: layout,
                    game: *mod_api::wire::LAYOUT,
                });
            }
            if instance
                .get_func(&mut group.store, "horizon_layout_ext")
                .is_some()
            {
                let built = instance
                    .get_typed_func::<(), u64>(&mut group.store, "horizon_layout_ext")
                    .map_err(load)?
                    .call(&mut group.store, ())
                    .map_err(load)?;
                if built != mod_points::POINT_LAYOUT {
                    return Err(Refusal::PointLayout {
                        built,
                        game: mod_points::POINT_LAYOUT,
                    });
                }
            }
            let input = instance
                .get_typed_func(&mut group.store, "horizon_in")
                .map_err(load)?;
            let hook = instance
                .get_typed_func(&mut group.store, "horizon_hook")
                .map_err(load)?;
            let mut prepared_entries = std::collections::HashMap::new();
            for (_, entries) in crate::boundary::PREPARED_EXPORTS {
                for (name, id) in entries {
                    if let Ok(entry) = instance.get_typed_func::<(), u64>(&mut group.store, name) {
                        prepared_entries.insert(*id, entry);
                    }
                }
            }
            exports.push((name, input, hook, Arc::new(prepared_entries)));
        }
        let lending = group.store.data().lending.clone();
        let running = Arc::new(Running {
            lending,
            group: Mutex::new(group),
            poisoned: AtomicBool::new(false),
            holder: Mutex::new(None),
        });
        running
            .group
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .store
            .data_mut()
            .group = Arc::as_ptr(&running) as usize;
        Ok(exports
            .into_iter()
            .map(|(name, input, hook, prepared_entries)| Self {
                id: name.into(),
                running: running.clone(),
                input,
                hook,
                subscribed: &[],
                listeners: &[],
                released: Arc::default(),
                prepared_releases: Arc::default(),
                prepared: Arc::default(),
                prepared_entries,
                subscriptions: Arc::default(),
            })
            .collect())
    }
    /// Refusals answer unavailable without stopping the group; faults stop all members.
    pub(crate) fn call<R>(
        &self,
        class: CallClass,
        frame: Frame,
        id: u32,
        handler: Option<Id>,
        args: impl FnOnce(&mut Writer),
        reply: impl FnOnce(&mut Reader<'_>) -> Result<R, WireError>,
    ) -> Option<R> {
        self.try_call(class, frame, id, handler, args, reply)
            .ok()
            .flatten()
    }

    fn try_call<R>(
        &self,
        class: CallClass,
        frame: Frame,
        id: u32,
        handler: Option<Id>,
        args: impl FnOnce(&mut Writer),
        reply: impl FnOnce(&mut Reader<'_>) -> Result<R, WireError>,
    ) -> Result<Option<R>, Busy> {
        // Dispatch-owned native receivers can unwind before our outer call
        // guards. Their destructors must not reenter the still-lent guest.
        if std::thread::panicking() {
            self.disable();
            note("called while the thread unwinds".into());
            return Ok(None);
        }
        if let Err(error) = horizon_guest_stack::check_current() {
            log::error!("wasm mod {}: {error}", self.mod_id());
            note(error.into());
            return Ok(None);
        }
        if self.running.poisoned.load(Ordering::Relaxed) {
            note("its group has stopped after an earlier fault".into());
            return Ok(None);
        }
        let key = Arc::as_ptr(&self.running) as usize;
        // A synchronous import can call any provider/handler, not just a named
        // callback trait. Such reentry shares the enclosing call's remaining budget.
        let mut resumed = Resumed::take(key);
        let mut locked = None;
        if resumed.is_none() {
            // Start the busy timeout on contention, not on the uncontended hot path.
            let mut start: Option<Instant> = None;
            loop {
                match self.running.group.try_lock() {
                    Ok(group) => {
                        locked = Some(group);
                        break;
                    }
                    Err(std::sync::TryLockError::Poisoned(e)) => {
                        locked = Some(e.into_inner());
                        break;
                    }
                    Err(_)
                        if class != CallClass::Worker
                            && (id == crate::prepared::RELEASE
                                // A nested cross-group call can form a lock cycle.
                                || (!LENT.get().is_null()
                                    && self.running.lending.load(Ordering::Relaxed) != 0)
                                || start.get_or_insert_with(Instant::now).elapsed()
                                    >= Duration::from_secs(5)) =>
                    {
                        if id != crate::prepared::RELEASE {
                            let why = self
                                .running
                                .busy(start.map_or(Duration::ZERO, |start| start.elapsed()));
                            log::warn!("wasm mod {} export {id}: {why}", self.id);
                            note(why);
                        }
                        return Err(Busy);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(1)),
                }
            }
        }
        if locked.is_some() {
            *self
                .running
                .holder
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(Holder {
                module: self.id.clone(),
                export: id,
                thread: std::thread::current(),
                since: Instant::now(),
            });
        }
        // Deferred releases run only on a top-level call: a resumed nested call is still
        // inside another export of the guest.
        let top = resumed.is_none();
        let mut store = match resumed.as_mut() {
            Some(resumed) => resumed.context(),
            None => {
                let store = &mut locked.as_mut().expect("group locked without a loan").store;
                let budget = if class == CallClass::Load {
                    30_000
                } else {
                    match CALL_BUDGET.load(Ordering::Relaxed) {
                        0 => 100,
                        budget => budget,
                    }
                };
                store.set_epoch_deadline(budget);
                store.data_mut().epoch_budget = budget;
                store.as_context_mut()
            }
        };
        if self.running.poisoned.load(Ordering::Relaxed) {
            if store.data().depth == 0 {
                drop(std::mem::take(&mut store.data_mut().cx.held));
            }
            note("its group has stopped after an earlier fault".into());
            return Ok(None);
        }
        let budget = store.data().epoch_budget;
        let depth = store.data().depth + 1;
        if depth > 64 {
            let path = format!("{:?} -> ({}, {id}), limit 64", store.data().path, self.id);
            if store.data_mut().refusals.insert(path.clone()) {
                log::warn!("wasm depth refusal: {path}");
            }
            note(format!("nested past depth 64: {path}"));
            return Ok(None);
        }
        store.data_mut().depth = depth;
        if store.data().replies.len() <= depth {
            store.data_mut().replies.resize_with(depth + 1, Writer::new);
            store.data_mut().inputs.resize_with(depth + 1, Writer::new);
            store
                .data_mut()
                .outputs
                .resize_with(depth + 1, || Vec::with_capacity(64));
            store.data_mut().imports.resize_with(depth + 1, Vec::new);
        }
        store.data_mut().cx.frames.push(Frame::default());
        store.data_mut().path.push((self.id.clone(), id));
        // self remains borrowed until this call returns. FrameScope restores the
        // enclosing module before this borrow ends, including on error or unwind.
        let previous_module =
            std::mem::replace(&mut store.data_mut().cx.module, self as *const Self);
        let scope = FrameScope {
            previous_module,
            state: store.data_mut(),
            running: self.running.clone(),
        };
        // SAFETY: FrameScope keeps the active module installed and the group
        // remains exclusively locked/lent until this resource scope is dropped.
        let resources = unsafe { crate::boundary::resource_scope(&mut *store.data_mut().cx, true) };
        let mut w = std::mem::take(&mut store.data_mut().inputs[depth]);
        let mut output = std::mem::take(&mut store.data_mut().outputs[depth]);
        // This guard unwinds before the resource scope, whose native destructors
        // must already see the group stopped.
        let panic_stop = PoisonOnPanic(&self.running.poisoned);
        let result = (|| -> Result<R, CallError> {
            if top {
                for (slot, generation) in self.prepared_releases.take() {
                    w.clear();
                    w.put(&slot);
                    w.put(&generation);
                    self.invoke(&mut store, crate::prepared::RELEASE, &w, &mut output)
                        .map_err(|error| {
                            CallError::Fault(
                                error.into_error().context("deferred prepared release"),
                            )
                        })?;
                    if !output.is_empty() {
                        return Err(CallError::Fault(
                            wasmtime::Error::from(WireError::Trailing)
                                .context("deferred prepared release"),
                        ));
                    }
                    self.finish_prepared_release(slot);
                }
            }
            // Deferred releases have the same empty frame as a direct release.
            *store.data_mut().cx.frames.last_mut().unwrap() = frame;
            let released = self.released.take();
            if !released.is_empty() {
                w.clear();
                w.put(&released);
                self.invoke(&mut store, mod_api::wire::RELEASE, &w, &mut output)?;
                if !output.is_empty() {
                    return Err(WireError::Trailing.into());
                }
            }
            w.clear();
            if let Some(handler) = handler {
                w.put(&handler);
            }
            args(&mut w);
            self.invoke(&mut store, id, &w, &mut output)?;
            let mut r = Reader::new(&output);
            let value = reply(&mut r)?;
            if !r.rest().is_empty() {
                return Err(WireError::Trailing.into());
            }
            Ok(value)
        })();
        store.data_mut().inputs[depth] = w;
        store.data_mut().outputs[depth] = output;
        // A failed guest release may leave retained state in the slot.
        let result = if id == crate::prepared::RELEASE {
            result.map_err(|error| CallError::Fault(error.into_error()))
        } else {
            result
        };
        let fault = match &result {
            Err(CallError::Fault(e)) => Some(fault_reason(e, budget, &mut store.data_mut().limits)),
            // A top-level call that ended without a fault handled any refused growth itself: a
            // later, unrelated fault must not be blamed on it.
            _ if depth == 1 => {
                store.data_mut().limits.refused_growth = None;
                None
            }
            _ => None,
        };
        if result.is_err() {
            horizon_mod_runtime::dispatch_failures::record_dispatch_failure();
        }
        let first_failure = matches!(result, Err(CallError::Fault(_)))
            && !self.running.poisoned.swap(true, Ordering::Relaxed);
        if let Err(CallError::Refused(error)) = &result {
            let reason = format!("{} hook {id}: {error:#}", self.id);
            if store
                .data_mut()
                .refusals
                .insert(format!("{} hook {id}", self.id))
            {
                log::warn!("wasm refusal: {reason}");
            }
        }
        drop(panic_stop);
        drop(resources);
        drop(scope);
        match result {
            Ok(value) => Ok(Some(value)),
            Err(CallError::Refused(e)) => {
                note(format!("refused: {e:#}"));
                Ok(None)
            }
            Err(CallError::Fault(e)) => {
                let reason = fault.unwrap_or_default();
                if first_failure {
                    log::error!(
                        "wasm mod {} stopped its group: {class:?}, depth {depth}, active epoch budget {budget}: {reason}: {e:#}",
                        self.id
                    );
                }
                note(reason);
                Ok(None)
            }
        }
    }
    fn finish_prepared_release(&self, slot: u32) {
        self.prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finish_release(slot);
    }

    fn invoke(
        &self,
        store: &mut StoreContextMut<'_, State>,
        id: u32,
        input: &Writer,
        output: &mut Vec<u8>,
    ) -> Result<(), CallError> {
        if let Some(error) = input.error() {
            return Err(error.into());
        }
        let len = u32::try_from(input.bytes().len()).map_err(|e| CallError::Refused(e.into()))?;
        let ptr = self
            .input
            .call(&mut *store, len)
            .map_err(CallError::Fault)?;
        let memory = store
            .data()
            .memory
            .ok_or_else(|| CallError::Fault(wasmtime::Error::msg("no memory")))?;
        memory
            .write(&mut *store, ptr as usize, input.bytes())
            .map_err(|e| CallError::Fault(e.into()))?;
        #[cfg(feature = "test-copy-probe")]
        crate::copy_probe::record(true, input.bytes().len());
        let result = if let Some(entry) = self.prepared_entries.get(&id) {
            entry.call(&mut *store, ()).map_err(CallError::Fault)?
        } else {
            self.hook.call(&mut *store, id).map_err(CallError::Fault)?
        };
        let ptr = (result >> 32) as usize;
        let len = result as u32;
        if len == u32::MAX {
            let bytes = memory.data(&*store).get(ptr..).ok_or_else(|| {
                CallError::Refused(wasmtime::Error::msg("error reply outside memory"))
            })?;
            let message: String = Reader::new(bytes).take()?;
            return Err(CallError::Refused(wasmtime::Error::msg(message)));
        }
        let end = ptr
            .checked_add(len as usize)
            .ok_or_else(|| CallError::Refused(wasmtime::Error::msg("reply overflow")))?;
        output.clear();
        output.extend_from_slice(
            memory
                .data(&*store)
                .get(ptr..end)
                .ok_or_else(|| CallError::Refused(wasmtime::Error::msg("reply outside memory")))?,
        );
        #[cfg(feature = "test-copy-probe")]
        crate::copy_probe::record(false, output.len());
        Ok(())
    }
}
