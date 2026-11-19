//! Links the members of one dependency group into one shared memory and one shared table.
//!
//! Members are position-independent core modules built with `rust-lld --shared`: each carries a
//! `dylink.0` section saying how much memory and table it needs, and imports `env.memory`,
//! `env.__memory_base`, `env.__table_base`, `env.__stack_pointer`, the functions of the members it
//! depends on (`env.<name>`), and the addresses of their data and functions (`GOT.mem.<name>`,
//! `GOT.func.<name>`). The algorithm is the one of the WebAssembly dynamic linking convention
//! (tool-conventions `DynamicLinking.md`): place each member, instantiate in dependency order, apply
//! its data relocations, then run every member's constructors. A member's functions and statics
//! are called directly by the members after it, with no crossing, and one copy of a library or of
//! std serves its whole group.
//!
//! Every module here comes from a mod: no input panics, and every refusal names the member.
use std::collections::BTreeMap;
use std::fmt;
use wasmparser::{KnownCustom, Parser, Payload};
use wasmtime::{
    Engine, Extern, ExternType, Global, GlobalType, Instance, Memory, MemoryType, Module,
    Mutability, RefType, Store, Table, TableType, Val, ValType,
};

const PAGE: u64 = 65536;
/// The shadow stack the members share, below their data.
const STACK: u64 = 256 * 1024;
/// Memory a group may start with, however large its members say their data is.
const MAX_STATIC: u64 = 1 << 30;
const MAX_TABLE: u64 = 1 << 20;

/// One member of a group, in dependency order: what it depends on comes first.
pub struct Member<'a> {
    pub name: &'a str,
    pub module: &'a [u8],
}

/// Why a group did not link or run, naming the member it concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// The member has no `dylink.0` section: it was not built as a shared module.
    NoDylink { member: String },
    /// The member's `dylink.0` or module is malformed.
    Malformed { member: String, why: String },
    /// The member asks for more memory or table than a group may have.
    TooLarge { member: String },
    /// The member imports a function or address no member before it provides.
    Unresolved { member: String, symbol: String },
    /// Two members export one name.
    Duplicate {
        symbol: String,
        first: String,
        second: String,
    },
    /// The member did not compile or instantiate (a type that doesn't match its provider's).
    Instantiate { member: String, why: String },
    /// A child compile exceeded its runtime budget; this is not malformed code.
    CompileTimeout {
        member: String,
        budget: crate::compile_budget::CompileBudget,
    },
    /// Bounded compilation was unavailable; never retry without the applied budget.
    CompileUnavailable {
        member: String,
        budget: crate::compile_budget::CompileBudget,
        why: String,
    },
    /// The shared std holds another group state than this game's (`words`), or predates the
    /// check and doesn't say (`None`).
    Std { member: String, words: Option<u32> },
    /// A call into a member trapped or named no export.
    Call {
        member: String,
        function: String,
        why: String,
    },
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDylink { member } => write!(f, "{member}: not a shared module (no dylink.0)"),
            Self::Malformed { member, why } => write!(f, "{member}: {why}"),
            Self::TooLarge { member } => write!(f, "{member}: needs more memory than a group has"),
            Self::Unresolved { member, symbol } => {
                write!(f, "{member}: imports {symbol}, which no earlier member has")
            }
            Self::Duplicate {
                symbol,
                first,
                second,
            } => write!(f, "{second}: exports {symbol}, which {first} already does"),
            Self::Instantiate { member, why } => write!(f, "{member}: {why}"),
            Self::CompileTimeout { member, budget } => {
                write!(f, "{member}: compilation time limit exceeded: {budget}")
            }
            Self::CompileUnavailable {
                member,
                budget,
                why,
            } => write!(
                f,
                "{member}: cannot enforce compilation budget {budget}: {why}; restore a private writable .horizon-cache/wasm directory and a working compiler executable"
            ),
            Self::Std {
                member,
                words: None,
            } => write!(
                f,
                "{member}: this shared std is older than the game and doesn't say how much group state it holds: reinstall the game"
            ),
            Self::Std {
                member,
                words: Some(words),
            } => write!(
                f,
                "{member}: this shared std holds {words} group-state words, the game uses {}: reinstall the game",
                mod_api::wire::GROUP_WORDS
            ),
            Self::Call {
                member,
                function,
                why,
            } => write!(f, "{member}: {function}: {why}"),
        }
    }
}

impl std::error::Error for LinkError {}

impl LinkError {
    pub(crate) fn compile(member: &str, error: wasmtime::Error) -> Self {
        if let Some(timeout) = error.downcast_ref::<crate::module_cache::CompileTimeout>() {
            Self::CompileTimeout {
                member: member.to_owned(),
                budget: timeout.0,
            }
        } else if let Some(unavailable) =
            error.downcast_ref::<crate::module_cache::CompileUnavailable>()
        {
            Self::CompileUnavailable {
                member: member.to_owned(),
                budget: unavailable.budget,
                why: unavailable.why.clone(),
            }
        } else {
            Self::Malformed {
                member: member.to_owned(),
                why: format!("{error:#}"),
            }
        }
    }
}

/// A linked group: its store, and each member's instance.
pub struct Group<T: 'static = ()> {
    pub(crate) store: Store<T>,
    pub(crate) members: Vec<(String, Instance)>,
    pub(crate) memory: Memory,
}

impl Group<()> {
    /// Links a group without host imports (useful for plain shared libraries).
    pub fn link(engine: &Engine, members: &[Member<'_>]) -> Result<Self, LinkError> {
        Self::link_with(Store::new(engine, ()), members, |_, _, _| None)
    }
}

impl<T: 'static> Group<T> {
    /// Links `members`, which must be in dependency order.
    pub fn link_with(
        store: Store<T>,
        members: &[Member<'_>],
        host: impl FnMut(&mut Store<T>, &str, &str) -> Option<Extern>,
    ) -> Result<Self, LinkError> {
        Self::link_cached_with(store, members, None, host)
    }

    pub(crate) fn link_cached_with(
        mut store: Store<T>,
        members: &[Member<'_>],
        cache: Option<&crate::module_cache::ModuleCache>,
        mut host: impl FnMut(&mut Store<T>, &str, &str) -> Option<Extern>,
    ) -> Result<Self, LinkError> {
        let _timer = crate::load_times::timer(crate::load_times::Phase::Group);
        let engine = store.engine();
        let mut parsed = Vec::with_capacity(members.len());
        for member in members {
            parsed.push(Layout::read(engine, member, cache)?);
        }
        let mut next = STACK;
        let mut table_next = 1u64;
        let mut got_funcs = 0u64;
        for (layout, member) in parsed.iter_mut().zip(members) {
            let size = |why: &str| LinkError::Malformed {
                member: member.name.to_owned(),
                why: why.to_owned(),
            };
            let align = 1u64
                .checked_shl(layout.info.memory_alignment)
                .filter(|align| *align <= PAGE)
                .ok_or_else(|| size("memory alignment past a page"))?;
            layout.memory_base = next.next_multiple_of(align);
            next = layout
                .memory_base
                .checked_add(u64::from(layout.info.memory_size))
                .filter(|end| *end <= MAX_STATIC)
                .ok_or_else(|| LinkError::TooLarge {
                    member: member.name.to_owned(),
                })?;
            let align = 1u64
                .checked_shl(layout.info.table_alignment)
                .filter(|align| *align <= MAX_TABLE)
                .ok_or_else(|| size("table alignment too large"))?;
            layout.table_base = table_next.next_multiple_of(align);
            table_next = layout
                .table_base
                .checked_add(u64::from(layout.info.table_size))
                .filter(|end| *end <= MAX_TABLE)
                .ok_or_else(|| LinkError::TooLarge {
                    member: member.name.to_owned(),
                })?;
            got_funcs += layout.got_funcs;
        }
        let load = |why: wasmtime::Error, member: &str| LinkError::Instantiate {
            member: member.to_owned(),
            why: format!("{why:#}"),
        };
        let memory = Memory::new(
            &mut store,
            MemoryType::new(next.div_ceil(PAGE) as u32, None),
        )
        .map_err(|e| load(e, "group"))?;
        let table_len = table_next + got_funcs;
        let table = Table::new(
            &mut store,
            TableType::new(RefType::FUNCREF, table_len as u32, None),
            wasmtime::Ref::Func(None),
        )
        .map_err(|e| load(e, "group"))?;
        let stack = Global::new(
            &mut store,
            GlobalType::new(ValType::I32, Mutability::Var),
            Val::I32(STACK as i32),
        )
        .map_err(|e| load(e, "group"))?;

        // Where each exported function and data symbol is, by name.
        let mut symbols: BTreeMap<String, (usize, Extern)> = BTreeMap::new();
        let mut instances: Vec<(String, Instance)> = Vec::with_capacity(members.len());
        let mut table_slot = table_next;
        let mut func_slots: BTreeMap<String, u32> = BTreeMap::new();
        for (index, (layout, member)) in parsed.iter().zip(members).enumerate() {
            let mut imports = Vec::new();
            let own_exports: BTreeMap<_, _> = layout
                .module
                .exports()
                .map(|e| (e.name().to_owned(), e.ty()))
                .collect();
            let mut self_funcs = BTreeMap::new();
            let mut self_data = Vec::new();
            for import in layout.module.imports() {
                let (module, name) = (import.module(), import.name());
                let unresolved = || LinkError::Unresolved {
                    member: member.name.to_owned(),
                    symbol: format!("{module}.{name}"),
                };
                let extern_ = if let Some(value) = host(&mut store, module, name) {
                    value
                } else {
                    match (module, name, import.ty()) {
                        ("env", "memory", _) => Extern::Memory(memory),
                        ("env", "__indirect_function_table", _) => Extern::Table(table),
                        ("env", "__stack_pointer", _) => Extern::Global(stack),
                        ("env", "__memory_base", _) => Extern::Global(
                            Global::new(
                                &mut store,
                                GlobalType::new(ValType::I32, Mutability::Const),
                                Val::I32(layout.memory_base as i32),
                            )
                            .map_err(|e| load(e, member.name))?,
                        ),
                        ("env", "__table_base", _) => Extern::Global(
                            Global::new(
                                &mut store,
                                GlobalType::new(ValType::I32, Mutability::Const),
                                Val::I32(layout.table_base as i32),
                            )
                            .map_err(|e| load(e, member.name))?,
                        ),
                        (
                            "GOT.mem" | "env",
                            "__heap_base" | "__data_end",
                            ExternType::Global(ty),
                        ) => Extern::Global(
                            Global::new(&mut store, ty, Val::I32(next.next_multiple_of(16) as i32))
                                .map_err(|e| load(e, member.name))?,
                        ),
                        ("GOT.mem" | "env", "__heap_end", ExternType::Global(ty)) => {
                            let end = memory.data_size(&store) as i32;
                            Extern::Global(
                                Global::new(&mut store, ty, Val::I32(end))
                                    .map_err(|e| load(e, member.name))?,
                            )
                        }
                        ("GOT.mem", symbol, _) => {
                            let Some((owner, Extern::Global(global))) = symbols
                                .get(symbol)
                                .filter(|_| !layout.private_exports.contains(symbol))
                            else {
                                if !matches!(own_exports.get(symbol), Some(ExternType::Global(_))) {
                                    return Err(unresolved());
                                }
                                let global = Global::new(
                                    &mut store,
                                    GlobalType::new(ValType::I32, Mutability::Var),
                                    Val::I32(0),
                                )
                                .map_err(|e| load(e, member.name))?;
                                self_data.push((symbol.to_owned(), global));
                                imports.push(Extern::Global(global));
                                continue;
                            };
                            let Val::I32(offset) = global.get(&mut store) else {
                                return Err(unresolved());
                            };
                            let address = parsed[*owner].memory_base as i64 + i64::from(offset);
                            Extern::Global(
                                Global::new(
                                    &mut store,
                                    GlobalType::new(ValType::I32, Mutability::Var),
                                    Val::I32(address as i32),
                                )
                                .map_err(|e| load(e, member.name))?,
                            )
                        }
                        ("GOT.func", symbol, _) => {
                            let Some((_, Extern::Func(func))) = symbols
                                .get(symbol)
                                .filter(|_| !layout.private_exports.contains(symbol))
                            else {
                                if !matches!(own_exports.get(symbol), Some(ExternType::Func(_))) {
                                    return Err(unresolved());
                                }
                                let slot = match self_funcs.get(symbol) {
                                    Some(slot) => *slot,
                                    None => {
                                        let slot = table_slot as u32;
                                        table_slot += 1;
                                        if !layout.private_exports.contains(symbol) {
                                            func_slots.insert(symbol.to_owned(), slot);
                                        }
                                        self_funcs.insert(symbol.to_owned(), slot);
                                        slot
                                    }
                                };
                                imports.push(Extern::Global(
                                    Global::new(
                                        &mut store,
                                        GlobalType::new(ValType::I32, Mutability::Var),
                                        Val::I32(slot as i32),
                                    )
                                    .map_err(|e| load(e, member.name))?,
                                ));
                                continue;
                            };
                            let slot = match func_slots.get(symbol) {
                                Some(slot) => *slot,
                                None => {
                                    let slot = table_slot as u32;
                                    table_slot += 1;
                                    table
                                        .set(&mut store, u64::from(slot), (*func).into())
                                        .map_err(|e| load(e, member.name))?;
                                    func_slots.insert(symbol.to_owned(), slot);
                                    slot
                                }
                            };
                            Extern::Global(
                                Global::new(
                                    &mut store,
                                    GlobalType::new(ValType::I32, Mutability::Var),
                                    Val::I32(slot as i32),
                                )
                                .map_err(|e| load(e, member.name))?,
                            )
                        }
                        ("env", symbol, ExternType::Func(_)) => match symbols.get(symbol) {
                            Some((_, found @ Extern::Func(_))) => found.clone(),
                            _ => return Err(unresolved()),
                        },
                        _ => return Err(unresolved()),
                    }
                };
                imports.push(extern_);
            }
            let instantiate = crate::load_times::timer(crate::load_times::Phase::Instantiate);
            let instance = Instance::new(&mut store, &layout.module, &imports).map_err(|e| {
                LinkError::Instantiate {
                    member: member.name.to_owned(),
                    why: format!("{e:#}"),
                }
            })?;
            drop(instantiate);
            // Shared members have no core start function. Resolve their self-references to
            // actual exports before any relocation helper or constructor can execute.
            for (name, slot) in self_funcs {
                let func =
                    instance
                        .get_func(&mut store, &name)
                        .ok_or_else(|| LinkError::Malformed {
                            member: member.name.to_owned(),
                            why: format!("self GOT.func {name} is not a function"),
                        })?;
                table
                    .set(&mut store, u64::from(slot), func.into())
                    .map_err(|e| load(e, member.name))?;
            }
            for (name, imported) in self_data {
                let exported =
                    instance
                        .get_global(&mut store, &name)
                        .ok_or_else(|| LinkError::Malformed {
                            member: member.name.to_owned(),
                            why: format!("self GOT.mem {name} is not a global"),
                        })?;
                let Val::I32(offset) = exported.get(&mut store) else {
                    return Err(LinkError::Malformed {
                        member: member.name.to_owned(),
                        why: format!("self GOT.mem {name} is not i32"),
                    });
                };
                let address = layout.memory_base as i64 + i64::from(offset);
                imported
                    .set(&mut store, Val::I32(address as i32))
                    .map_err(|e| load(e, member.name))?;
            }
            let exports: Vec<_> = instance
                .exports(&mut store)
                .map(|export| (export.name().to_owned(), export.into_extern()))
                .collect();
            for (name, extern_) in exports {
                if layout.private_exports.contains(&name)
                    || name.starts_with("__wasm_")
                    || matches!(
                        name.as_str(),
                        "horizon_layout" | "horizon_layout_ext" | "horizon_in" | "horizon_hook"
                    )
                    || crate::boundary::PREPARED_EXPORTS
                        .iter()
                        .any(|(_, entries)| entries.iter().any(|(entry, _)| *entry == name))
                    || name == "memory"
                    || !matches!(extern_, Extern::Func(_) | Extern::Global(_))
                {
                    continue;
                }
                if let Some((first, _)) = symbols.get(&name) {
                    return Err(LinkError::Duplicate {
                        symbol: name,
                        first: members[*first].name.to_owned(),
                        second: member.name.to_owned(),
                    });
                }
                symbols.insert(name, (index, extern_));
            }
            instances.push((member.name.to_owned(), instance));
        }
        let mut group = Self {
            store,
            members: instances,
            memory,
        };
        // The std member holds the state every member shares: its size is checked before any
        // relocation or constructor can touch it. Its answer is a constant, needing neither.
        for (name, instance) in &group.members {
            if instance
                .get_export(&mut group.store, "horizon_group_state")
                .is_none()
            {
                continue;
            }
            let words =
                match instance.get_typed_func::<(), u32>(&mut group.store, "horizon_group_words") {
                    Err(_) => None,
                    Ok(words) => {
                        Some(
                            words
                                .call(&mut group.store, ())
                                .map_err(|e| LinkError::Call {
                                    member: name.clone(),
                                    function: "horizon_group_words".into(),
                                    why: format!("{e:#}"),
                                })?,
                        )
                    }
                };
            if words.is_none_or(|words| words as usize != mod_api::wire::GROUP_WORDS) {
                return Err(LinkError::Std {
                    member: name.clone(),
                    words,
                });
            }
        }
        let _instantiate = crate::load_times::timer(crate::load_times::Phase::Instantiate);
        for hook in [
            "__wasm_start",
            "__wasm_apply_data_relocs",
            "__wasm_call_ctors",
        ] {
            for index in 0..group.members.len() {
                let name = group.members[index].0.clone();
                if let Some(func) = group.members[index].1.get_func(&mut group.store, hook) {
                    func.call(&mut group.store, &[], &mut [])
                        .map_err(|e| LinkError::Call {
                            member: name,
                            function: hook.to_owned(),
                            why: format!("{e:#}"),
                        })?;
                }
            }
        }
        Ok(group)
    }

    /// Calls `member`'s exported function `function`.
    pub fn call(
        &mut self,
        member: &str,
        function: &str,
        params: &[Val],
        results: &mut [Val],
    ) -> Result<(), LinkError> {
        let fail = |why: String| LinkError::Call {
            member: member.to_owned(),
            function: function.to_owned(),
            why,
        };
        let instance = self
            .members
            .iter()
            .find(|(name, _)| name == member)
            .map(|(_, instance)| *instance)
            .ok_or_else(|| fail("no such member".into()))?;
        let func = instance
            .get_func(&mut self.store, function)
            .ok_or_else(|| fail("no such export".into()))?;
        func.call(&mut self.store, params, results)
            .map_err(|e| fail(format!("{e:#}")))
    }
}

/// A member read: its module, what it asks for, and where the group put it.
struct Layout {
    module: Module,
    info: wasmparser::MemInfo,
    got_funcs: u64,
    private_exports: std::collections::BTreeSet<String>,
    memory_base: u64,
    table_base: u64,
}

impl Layout {
    fn read(
        engine: &Engine,
        member: &Member<'_>,
        cache: Option<&crate::module_cache::ModuleCache>,
    ) -> Result<Self, LinkError> {
        let _timer = crate::load_times::timer(crate::load_times::Phase::Modules);
        let malformed = |why: String| LinkError::Malformed {
            member: member.name.to_owned(),
            why,
        };
        let mut info = None;
        let mut seen = false;
        let mut private_exports = None;
        for payload in Parser::new(0).parse_all(member.module) {
            let payload = payload.map_err(|e| malformed(e.to_string()))?;
            match &payload {
                Payload::StartSection { .. } => return Err(malformed(
                    "shared members run constructors after relocation, not a core start function"
                        .into(),
                )),
                Payload::MemorySection(section) if section.count() != 0 => {
                    return Err(malformed(
                        "a shared member must import the group memory".into(),
                    ));
                }
                Payload::TableSection(section) if section.count() != 0 => {
                    return Err(malformed(
                        "a shared member must import the group table".into(),
                    ));
                }
                _ => {}
            }
            if let Payload::CustomSection(section) = &payload
                && section.name() == "horizon.private"
            {
                if private_exports.is_some() {
                    return Err(malformed("duplicate horizon.private section".into()));
                }
                private_exports = Some(
                    serde_json::from_slice::<std::collections::BTreeSet<String>>(section.data())
                        .map_err(|e| malformed(format!("private exports: {e}")))?,
                );
            }
            if let Payload::CustomSection(section) = payload
                && section.name() == "dylink.0"
            {
                seen = true;
                let KnownCustom::Dylink0(reader) = section.as_known() else {
                    return Err(malformed("dylink.0 is unreadable".into()));
                };
                for subsection in reader {
                    if let wasmparser::Dylink0Subsection::MemInfo(found) =
                        subsection.map_err(|e| malformed(e.to_string()))?
                    {
                        info = Some(found);
                    }
                }
            }
        }
        if !seen {
            return Err(LinkError::NoDylink {
                member: member.name.to_owned(),
            });
        }
        let info = info.ok_or_else(|| malformed("dylink.0 has no memory info".into()))?;
        let module = match cache {
            Some(cache) => cache
                .load_or_compile(engine, member.module)
                .map(|(module, _)| module),
            None => Module::new(engine, member.module),
        }
        .map_err(|error| LinkError::compile(member.name, error))?;
        let got_funcs = module
            .imports()
            .filter(|import| import.module() == "GOT.func")
            .count() as u64;
        Ok(Self {
            module,
            info,
            got_funcs,
            private_exports: private_exports.unwrap_or_default(),
            memory_base: 0,
            table_base: 0,
        })
    }
}
