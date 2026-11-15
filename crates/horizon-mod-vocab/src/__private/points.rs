//! The engine API point registry, shared by native and wasm listeners.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Runs a point's listened path out of line: the code that builds its record and calls its
/// listeners stays out of the function, which nothing listening leaves small enough to inline.
#[cold]
#[inline(never)]
pub fn cold<R>(path: impl FnOnce() -> R) -> R {
    path()
}

/// One injection point table: its API point slots.
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub struct Table {
    points: &'static [AtomicBool],
    sealed: OnceLock<Sealed>,
    /// Each place's target function, when this build compiled no sites for the places: a
    /// listener there would never run, so [`register`] refuses it by this name.
    unsited: Option<&'static [&'static str]>,
}

impl Table {
    /// A table over its API point slots, whose sites are compiled into this build.
    pub const fn new(points: &'static [AtomicBool]) -> Self {
        Self {
            points,
            sealed: OnceLock::new(),
            unsited: None,
        }
    }

    /// A table whose places have no site in this build: `targets` names each place's function,
    /// and [`register`] refuses every listener.
    pub const fn without_sites(
        points: &'static [AtomicBool],
        targets: &'static [&'static str],
    ) -> Self {
        Self {
            points,
            sealed: OnceLock::new(),
            unsited: Some(targets),
        }
    }

    /// Whether this build compiled the places' sites, so a listener registered here can run.
    pub fn has_sites(&self) -> bool {
        self.unsited.is_none()
    }

    /// Runs the API functions and forwarding endpoints at point `place`, each under its owner's
    /// guard.
    #[cold]
    #[inline(never)]
    pub fn fire_api(&self, place: usize, call: &mut dyn FnMut(ApiKind)) {
        let Some(sealed) = self.sealed.get() else {
            return;
        };
        let slot = &sealed.slots[place];
        let entries = &slot.entries;
        for entry in entries {
            let listener = &sealed.listeners[entry.listener as usize];
            if listener.active() {
                let _guard = super::enter(listener.id, entry.injection);
                call(entry.kind);
            }
        }
    }

    /// Sets the table's slots from its registrations: each place's listeners in order.
    fn seal(&self, mut listeners: Vec<Owner>, slots: Vec<(usize, Slot)>, stopped: &[String]) {
        for listener in &mut listeners {
            listener.mod_index = super::mod_index(listener.id);
            if stopped.iter().any(|id| id == listener.id) {
                listener.stopped.store(true, Ordering::Release);
            }
        }
        let points = self.points.len();
        let mut lists = vec![Vec::new(); points];
        for (place, slot) in slots {
            lists[place].push(slot);
        }
        for list in &mut lists {
            list.sort_by_key(|entry| (entry.replace, std::cmp::Reverse(entry.priority)));
            for group in list.chunk_by_mut(|a, b| {
                a.replace == b.replace && a.priority == b.priority && a.listener == b.listener
            }) {
                group.sort_by_key(|entry| entry.injection);
            }
        }
        let sealed = self.sealed.get_or_init(|| Sealed {
            listeners: listeners.into(),
            slots: lists
                .into_iter()
                .map(|entries| SlotList {
                    entries: entries.into_boxed_slice(),
                })
                .collect(),
        });
        for (slot, list) in self.points.iter().zip(&sealed.slots) {
            slot.store(!list.entries.is_empty(), Ordering::Release);
        }
    }
}

/// A place in a table: a point's head or return, only.
#[derive(Clone, Copy)]
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub struct Place {
    table: &'static Table,
    index: usize,
}

impl Place {
    /// Place `index` of `table`.
    pub const fn new(table: &'static Table, index: usize) -> Self {
        Self { table, index }
    }

    /// The table the place is in.
    pub fn table(self) -> &'static Table {
        self.table
    }

    /// Its index in its table.
    pub fn index(self) -> usize {
        self.index
    }
}

struct Owner {
    id: &'static str,
    /// Resolved at the seal, so firing reads the owner's canonical enable flag without a scan.
    mod_index: Option<usize>,
    /// Set when the mod stopped (a trap in its wasm group, or the game giving up on it): its
    /// listeners are skipped from then on.
    stopped: AtomicBool,
}

impl Owner {
    fn active(&self) -> bool {
        !self.stopped.load(Ordering::Acquire) && self.mod_index.is_none_or(super::mod_index_on)
    }
}

/// The host available to a point's hook call; frame objects never cross as data.
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub enum PointHost<'a> {
    /// A read-only point.
    Read(&'a dyn crate::Host),
    /// A server write point.
    Server(&'a mut dyn crate::ServerHost),
    /// A client write point.
    Client(&'a mut dyn crate::ClientHost),
    /// A point which runs on either side.
    Side(crate::HostSide<'a>),
    /// A free function without a host.
    None,
}

/// A checked hook endpoint. The runtime calls the ordinary hook export under its limits.
pub trait Forward: Send + Sync {
    /// Dispatches one generated listener id with its mutable wire record.
    fn forward(&self, id: u32, host: PointHost<'_>, record: &mut dyn crate::wire::Record);
}

/// A typed API function.
#[derive(Clone, Copy)]
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub enum ApiKind {
    /// A function checked by the generated typed constructor before erasure.
    Typed(*const ()),
    /// A runtime endpoint in the same ordered list as native functions.
    Forwarded(&'static dyn Forward, u32),
}

// The pointer is a function; forwarding endpoints are Send + Sync resources.
#[allow(
    unsafe_code,
    reason = "Carried point-registry function-pointer erasure; ownership cleanup is tracked in #2587"
)]
unsafe impl Send for ApiKind {}
#[allow(
    unsafe_code,
    reason = "Carried point-registry function-pointer erasure; ownership cleanup is tracked in #2587"
)]
unsafe impl Sync for ApiKind {}

/// A listener in its place's list.
#[derive(Clone, Copy)]
struct Slot {
    listener: u32,
    kind: ApiKind,
    priority: i32,
    injection: &'static str,
    replace: bool,
}

struct SlotList {
    entries: Box<[Slot]>,
}

struct Sealed {
    listeners: Box<[Owner]>,
    slots: Box<[SlotList]>,
}

/// One table's registrations while mods load.
struct OpenTable {
    table: &'static Table,
    listeners: Vec<Owner>,
    slots: Vec<(usize, Slot)>,
}

/// Engine point registrations while mods load, and stops that came before the seal.
struct Open {
    table: Option<OpenTable>,
    stopped: Vec<String>,
    /// Set once [`seal`] has run: registering is refused from then on, and stops go to the
    /// sealed engine table.
    sealed: bool,
}

/// Registering, stopping and sealing all hold this lock, so a registration either lands in the
/// sealed lists or is refused, and a stop before the seal still holds after it.
static OPEN: Mutex<Open> = Mutex::new(Open {
    table: None,
    stopped: Vec::new(),
    sealed: false,
});

fn lock() -> std::sync::MutexGuard<'static, Open> {
    OPEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Why a listener wasn't registered.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
pub enum RegisterError {
    /// The list was sealed: listeners register while the game loads, before its points are set.
    Sealed,
    /// A game listener requires a staged build with point sites.
    Unstaged {
        /// Owning mod.
        id: &'static str,
        /// Target function.
        target: &'static str,
    },
    /// The place is outside its table.
    NoPlace {
        /// Owning mod.
        id: &'static str,
        /// The place's index.
        index: usize,
    },
    /// The guest table requires a typed API listener.
    Unsupported,
    /// A second `replace` listener at a place that already has one: two overwrites can't both win,
    /// as with a mixin overwrite conflict. Names the mod and injection of each.
    Replaced {
        /// The mod and injection id already replacing the function.
        first: (&'static str, &'static str),
        /// The mod and injection id refused.
        second: (&'static str, &'static str),
    },
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unstaged { id, target } => write!(
                f,
                "{id}: {target}: this build has no point sites; start the game through the launcher's mod build"
            ),
            Self::NoPlace { id, index } => {
                write!(f, "{id}: place {index} is outside its point table")
            }
            Self::Sealed => f.write_str("the game's points were already set"),
            Self::Unsupported => f.write_str("the guest table requires a typed API listener"),
            Self::Replaced { first, second } => write!(
                f,
                "{}:{} replaces a function that {}:{} already replaces; only one replace \
                 listener can own it",
                second.0, second.1, first.0, first.1
            ),
        }
    }
}

impl std::error::Error for RegisterError {}

/// A function or checked endpoint at an API slot.
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub struct SlotListener {
    id: &'static str,
    place: Place,
    kind: ApiKind,
    priority: i32,
    injection: &'static str,
    /// A `replace` listener: the function is its, so a second one at the place is refused.
    replace: bool,
}

impl SlotListener {
    /// An API listener whose function has the place's exact generated function type.
    ///
    /// # Safety
    /// The generated constructor must check the function's host and record types before erasure.
    #[allow(
        unsafe_code,
        reason = "Carried typed point registration contract; ownership cleanup is tracked in #2587"
    )]
    pub const unsafe fn api(id: &'static str, place: Place, function: *const ()) -> Self {
        Self::of(id, place, ApiKind::Typed(function))
    }

    /// A checked runtime endpoint at an API place, dispatched through the hook machinery.
    pub const fn forwarded(
        id: &'static str,
        place: Place,
        endpoint: &'static dyn Forward,
        export: u32,
    ) -> Self {
        Self::of(id, place, ApiKind::Forwarded(endpoint, export))
    }

    const fn of(id: &'static str, place: Place, kind: ApiKind) -> Self {
        Self {
            id,
            place,
            kind,
            priority: 0,
            injection: "",
            replace: false,
        }
    }

    /// Orders a point by priority, then dependency order, then inject id within its mod.
    pub const fn ordered(mut self, priority: i32, injection: &'static str) -> Self {
        self.priority = priority;
        self.injection = injection;
        self
    }

    /// Marks a `replace` listener: registering a second one at the same place is refused with
    /// [`RegisterError::Replaced`].
    pub const fn replacing(mut self) -> Self {
        self.replace = true;
        self
    }

    /// The function, when it is a typed API listener.
    pub fn api_kind(&self) -> Option<ApiKind> {
        Some(self.kind)
    }

    /// Where it listens.
    pub fn place(&self) -> Place {
        self.place
    }

    /// Its inject id.
    pub fn injection(&self) -> &'static str {
        self.injection
    }

    /// The mod's id.
    pub fn id(&self) -> &'static str {
        self.id
    }
}

/// Registers a function or checked endpoint in its place's table, in the same order for native
/// and wasm owners. The one boundary every listener passes: a table whose places have no site in
/// this build refuses here, naming the mod and the target, so no listener registers and never runs.
pub fn register(slot: &SlotListener) -> Result<(), RegisterError> {
    // A place is built from any table and index: one outside its table is refused, not indexed.
    let table = slot.place.table;
    let outside = RegisterError::NoPlace {
        id: slot.id,
        index: slot.place.index,
    };
    if slot.place.index >= table.points.len() {
        return Err(outside);
    }
    if let Some(targets) = table.unsited {
        let Some(target) = targets.get(slot.place.index) else {
            return Err(outside);
        };
        return Err(RegisterError::Unstaged {
            id: slot.id,
            target,
        });
    }
    let mut open = lock();
    if open.sealed {
        return Err(RegisterError::Sealed);
    }
    let table = open.table.get_or_insert_with(|| OpenTable {
        table: slot.place.table,
        listeners: Vec::new(),
        slots: Vec::new(),
    });
    if !std::ptr::eq(table.table, slot.place.table) {
        return Err(RegisterError::Unsupported);
    }
    let place = slot.place.index;
    if slot.replace
        && let Some((_, first)) = table
            .slots
            .iter()
            .find(|(at, other)| *at == place && other.replace)
    {
        return Err(RegisterError::Replaced {
            first: (table.listeners[first.listener as usize].id, first.injection),
            second: (slot.id, slot.injection),
        });
    }
    let listener = match table.listeners.iter().position(|owner| owner.id == slot.id) {
        Some(index) => index,
        None => {
            table.listeners.push(Owner {
                id: slot.id,
                mod_index: None,
                stopped: AtomicBool::new(false),
            });
            table.listeners.len() - 1
        }
    } as u32;
    table.slots.push((
        place,
        Slot {
            listener,
            kind: slot.kind,
            priority: slot.priority,
            injection: slot.injection,
            replace: slot.replace,
        },
    ));
    Ok(())
}

/// Stops mod `id`'s engine listeners: none of them runs again, whether the slots are
/// sealed yet or not. A trap that stops a wasm group and the game giving up on a native mod both
/// end here.
pub fn stop(id: &str) {
    let mut open = lock();
    if !open.sealed {
        open.stopped.push(id.to_owned());
        return;
    }
    for table in open.table.iter() {
        let Some(sealed) = table.table.sealed.get() else {
            continue;
        };
        for listener in sealed.listeners.iter().filter(|listener| listener.id == id) {
            listener.stopped.store(true, Ordering::Release);
        }
    }
}

/// Seals the engine table, refusing later registrations, and sets listened slots. The
/// game calls it once, after the mods load.
pub fn seal() {
    let mut open = lock();
    if open.sealed {
        return;
    }
    open.sealed = true;
    let stopped = std::mem::take(&mut open.stopped);
    for table in open.table.iter_mut() {
        let (listeners, slots) = (
            std::mem::take(&mut table.listeners),
            std::mem::take(&mut table.slots),
        );
        table.table.seal(listeners, slots, &stopped);
    }
}

/// Adapts a replacement listener to a point's ordinary typed listener without registry values.
// hidden: generated engine point adapters share replacement semantics.
#[doc(hidden)]
#[macro_export]
macro_rules! replacement_listener {
    (cancel, host, $listener:path) => {
        |host, args| {
            $listener(host, args);
            args.cancel = true;
        }
    };
    (cancel, none, $listener:path) => {
        |args| {
            $listener(args);
            args.cancel = true;
        }
    };
    (result, host, $listener:path) => {
        |host, args| {
            args.result = ::core::option::Option::Some($listener(host, args));
        }
    };
    (result, none, $listener:path) => {
        |args| {
            args.result = ::core::option::Option::Some($listener(args));
        }
    };
}
