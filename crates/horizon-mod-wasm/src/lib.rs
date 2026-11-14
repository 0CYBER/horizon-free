//! The runtime for wasm mods. Packages declaring `[wasm]` are grouped by their `[dependencies]`
//! (the same model as native mods: required and optional, semver on `horizon-mod.toml`
//! versions, one library copy shared by its dependents). Every refusal names its reason for the
//! Mods page.
pub mod compile_budget;
#[cfg(all(test, unix))]
mod compile_budget_tests;
mod compile_child;
#[cfg(feature = "test-copy-probe")]
pub mod copy_probe;
#[cfg(feature = "runtime-fixture")]
pub mod fixture;
pub mod link;
mod module_cache;
pub use compile_child::compile_child;
#[cfg(test)]
#[path = "../tests/support/no_hooks.rs"]
mod no_hooks;
mod points;
mod prepared;
mod registration_capabilities;
mod runtime;
// Why this thread's last guest call answered nothing, for the generated proxies' errors.
use runtime::take_refusal as wasm_call_refusal;
#[allow(
    dead_code,
    unused_variables,
    unused_mut,
    unsafe_op_in_unsafe_fn,
    clippy::all
)]
pub(crate) mod boundary {
    include!(concat!(env!("OUT_DIR"), "/wire_host_generated.rs"));
}
pub(crate) use runtime::WeakWasmMod;
pub use runtime::{CallClass, WasmHandler, WasmMod};

#[cfg(test)]
mod test_log;
#[cfg(test)]
mod test_stack;

pub(crate) trait NoAnswer {
    fn no_answer() -> Self;
}
impl<T: Default> NoAnswer for T {
    fn no_answer() -> Self {
        T::default()
    }
}
pub(crate) fn call_refused<E: From<(mod_api::Identifier, String)>>() -> E {
    E::from((
        mod_api::Identifier::parse("horizon:wasm_mod").expect("valid constant identifier"),
        "the mod's call didn't run".into(),
    ))
}

/// A hook a wasm mod may name but cannot be served yet: the load refuses by
/// name with the reason, as `alter_ground`'s inline check does, never a
/// silent skip past a subscription nothing would ever call.
pub(crate) fn native_only_hook(name: &str) -> Option<&'static str> {
    match name {
        // Stage 4a of #792 ships the general entity tick's native dispatch
        // only; the wasm adapter is stage 4b.
        "general_entity_tick" => Some(
            "general_entity_tick: general entity ticks are not yet available to wasm mods (the wasm adapter is #792's stage 4b)",
        ),
        _ => None,
    }
}

use horizon_mod_package::dependencies;
use horizon_mod_package::manifest::Manifest;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

/// One wasm mod as installed: its manifest and its checked `mod.wasm`.
pub struct WasmPackage {
    pub manifest: Manifest,
    pub module: Vec<u8>,
}

/// Why a wasm mod doesn't load, for the Mods page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A required dependency isn't installed, or none installed matches its version.
    MissingDependency { dependency: String, version: String },
    /// The shared native/wasm resolver refused the dependency graph.
    DependencyOrder { reason: String },
    /// A dependency's installed `mod.wasm` isn't the one this mod was linked against.
    LinkedAgainst { dependency: String },
    /// Members of one group were built with different toolchains.
    Toolchain { found: String, group: String },
    /// A dependency of this mod was refused, so its group can't link.
    DependencyRefused { dependency: String },
    /// The mod declares engine internals, which wasm cannot access.
    Internals,
    /// The package's `mod.wasm` failed its checks.
    Package(String),
    /// The group could not instantiate, initialize or run.
    Runtime(String),
    /// The configured compilation setting or its file could not be applied.
    CompileSetting(compile_budget::SettingError),
    /// A module exceeded the compilation budget, without poisoning its cache.
    CompileTimeout {
        member: String,
        budget: compile_budget::CompileBudget,
    },
    /// Compilation could not run with its required deadline.
    CompileUnavailable {
        member: String,
        budget: compile_budget::CompileBudget,
        why: String,
    },
    /// The mod was compiled against another generated boundary.
    Layout { built: u64, game: u64 },
    /// The mod's generated point records differ from this game's.
    PointLayout { built: u64, game: u64 },
    /// The game has no shared Rust standard library beside its executable.
    NoStd,
    /// The group's mods declare more memory (`wasm_memory_mib`) than this instance's cap.
    Memory {
        group: String,
        declared_mib: u32,
        cap_mib: u32,
    },
    /// A running group trapped or exhausted a resource limit.
    Stopped,
    /// Another thread owns a group whose import is running native code.
    Busy,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDependency {
                dependency,
                version,
            } => {
                write!(f, "needs {dependency} {version}, which isn't installed")
            }
            Self::DependencyOrder { reason } => write!(f, "{reason}"),
            Self::LinkedAgainst { dependency } => {
                write!(f, "built against another build of {dependency}")
            }
            Self::Toolchain { found, group } => {
                write!(f, "built with {found}, its dependencies with {group}")
            }
            Self::DependencyRefused { dependency } => {
                write!(f, "its dependency {dependency} didn't load")
            }
            Self::Internals => write!(f, "internals access is native-only"),
            Self::Package(why) => write!(f, "{why}"),
            Self::Runtime(why) => write!(f, "{why}"),
            Self::CompileSetting(why) => write!(f, "{why}"),
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
            Self::Layout { .. } => write!(
                f,
                "built for another version of the mod API (including the prepared can_equip ABI)"
            ),
            Self::PointLayout { built, game } => {
                write!(
                    f,
                    "built for another version of the mod points API (mod layout {built}, game layout {game})"
                )
            }
            Self::NoStd => write!(f, "this game has no wasm standard library member"),
            Self::Memory {
                group,
                declared_mib,
                cap_mib,
            } => write!(
                f,
                "wasm group {group} declares {declared_mib} MiB of memory (wasm_memory_mib), above this instance's cap of {cap_mib} MiB (wasm.memory_cap_mib in horizon-mods.txt)"
            ),
            Self::Stopped => write!(f, "stopped after a trap or resource limit"),
            Self::Busy => write!(f, "its wasm group is busy"),
        }
    }
}

impl Refusal {
    pub(crate) fn from_link(error: link::LinkError) -> Self {
        let module_file = |member: String| {
            member
                .strip_prefix("lib")
                .and_then(|name| name.strip_suffix(".so"))
                .filter(|name| *name != "std")
                .map_or_else(
                    || "horizon-wasm-std.wasm".to_owned(),
                    |id| format!("{id}:mod.wasm"),
                )
        };
        match error {
            link::LinkError::CompileTimeout { member, budget } => Self::CompileTimeout {
                member: module_file(member),
                budget,
            },
            link::LinkError::CompileUnavailable {
                member,
                budget,
                why,
            } => Self::CompileUnavailable {
                member: module_file(member),
                budget,
                why,
            },
            other => Self::Runtime(other.to_string()),
        }
    }
}

impl std::error::Error for Refusal {}

/// Splits the packages into groups: each mod with everything it depends on, transitively;
/// two mods sharing a dependency share its group, so a library has one copy. A mod whose
/// required dependency is missing, or whose linked hashes or toolchain disagree, is refused
/// with every mod depending on it.
fn groups(
    packages: &[WasmPackage],
    side: horizon_mod_package::manifest::Side,
) -> (Vec<Vec<usize>>, Vec<(String, Refusal)>) {
    let by_id: BTreeMap<&str, usize> = packages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.manifest.id.as_str(), i))
        .collect();
    let mut refused: BTreeMap<usize, Refusal> = BTreeMap::new();
    let declared: Vec<_> = packages
        .iter()
        .map(|p| dependencies::from_manifest(&p.manifest.dependencies, side))
        .collect();
    // Engine internals are native-only. A mod dependency's internals stay inside the group.
    for (i, package) in packages.iter().enumerate() {
        if !package.manifest.internals.is_empty() {
            refused.insert(i, Refusal::Internals);
        }
    }
    propagate_refusals(&declared, &by_id, &mut refused);
    // Physical library ordering cannot be dropped or reversed by a manifest
    // dispatch override. Joint host validation owns version/rule checks; the
    // actual linked hashes and toolchains are still checked below.
    let order = loop {
        let active: Vec<_> = (0..packages.len())
            .filter(|i| !refused.contains_key(i))
            .collect();
        let inputs: Vec<_> = active
            .iter()
            .map(|&i| dependencies::Mod {
                id: &packages[i].manifest.id,
                version: &packages[i].manifest.version,
                has_api: true,
                code: true,
                dependencies: &declared[i],
            })
            .collect();
        match dependencies::resolve_links(&inputs) {
            Ok(resolution) => {
                break resolution
                    .order
                    .into_iter()
                    .map(|i| active[i])
                    .collect::<Vec<_>>();
            }
            Err(error) => {
                let reason = error.to_string();
                let owners: Vec<&str> = match &error {
                    dependencies::Error::Cycle { mods } => {
                        mods.iter().map(String::as_str).collect()
                    }
                    dependencies::Error::Duplicate(id) => vec![id],
                    dependencies::Error::Manifest { owner, .. }
                    | dependencies::Error::Missing { owner, .. }
                    | dependencies::Error::NoApi { owner, .. }
                    | dependencies::Error::CargoConflict { owner, .. }
                    | dependencies::Error::Version { owner, .. }
                    | dependencies::Error::Incompatible { owner, .. } => vec![owner],
                };
                for &i in &active {
                    if owners.contains(&packages[i].manifest.id.as_str()) {
                        let refusal = match &error {
                            dependencies::Error::Missing { target, .. }
                            | dependencies::Error::Version { target, .. } => {
                                Refusal::MissingDependency {
                                    dependency: target.clone(),
                                    version: declared[i]
                                        .iter()
                                        .find(|d| d.id == *target)
                                        .unwrap()
                                        .requirement
                                        .to_string(),
                                }
                            }
                            _ => Refusal::DependencyOrder {
                                reason: reason.clone(),
                            },
                        };
                        refused.insert(i, refusal);
                    }
                }
                propagate_refusals(&declared, &by_id, &mut refused);
            }
        }
    };

    for (i, package) in packages.iter().enumerate() {
        let wasm = package
            .manifest
            .wasm
            .as_ref()
            .expect("a wasm package declares [wasm]");
        for id in declared[i].iter().map(|dependency| &dependency.id) {
            let Some(&d) = by_id.get(id.as_str()) else {
                continue;
            };
            let dep = packages[d]
                .manifest
                .wasm
                .as_ref()
                .expect("a wasm package declares [wasm]");
            if wasm
                .linked()
                .get(id)
                .is_some_and(|hash| hash != dep.sha512())
            {
                refused.entry(i).or_insert(Refusal::LinkedAgainst {
                    dependency: id.clone(),
                });
            }
            if dep.toolchain() != wasm.toolchain() {
                refused.entry(i).or_insert(Refusal::Toolchain {
                    found: wasm.toolchain().into(),
                    group: dep.toolchain().into(),
                });
            }
        }
    }
    propagate_refusals(&declared, &by_id, &mut refused);
    // Union-find over the dependency edges of the mods that stay.
    let mut parent: Vec<usize> = (0..packages.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (i, dependencies) in declared
        .iter()
        .enumerate()
        .filter(|(i, _)| !refused.contains_key(i))
    {
        for id in dependencies.iter().map(|dependency| &dependency.id) {
            if let Some(&d) = by_id.get(id.as_str()).filter(|d| !refused.contains_key(d)) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, d));
                parent[a] = b;
            }
        }
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in (0..packages.len()).filter(|i| !refused.contains_key(i)) {
        by_root.entry(root(&mut parent, i)).or_default().push(i);
    }
    let groups = by_root
        .into_values()
        .map(|members| {
            order
                .iter()
                .copied()
                .filter(|i| members.contains(i))
                .collect()
        })
        .collect();
    let refused = refused
        .into_iter()
        .map(|(i, r)| (packages[i].manifest.id.clone(), r))
        .collect();
    (groups, refused)
}

/// A refused mod takes its dependents with it, until nothing changes.
fn propagate_refusals(
    declared: &[Vec<dependencies::Dependency>],
    by_id: &BTreeMap<&str, usize>,
    refused: &mut BTreeMap<usize, Refusal>,
) {
    loop {
        let before = refused.len();
        for (i, dependencies) in declared.iter().enumerate() {
            if refused.contains_key(&i) {
                continue;
            }
            if let Some(id) = dependencies
                .iter()
                .map(|dependency| &dependency.id)
                .find(|id| {
                    by_id
                        .get(id.as_str())
                        .is_some_and(|d| refused.contains_key(d))
                })
            {
                refused.insert(
                    i,
                    Refusal::DependencyRefused {
                        dependency: id.clone(),
                    },
                );
            }
        }
        if refused.len() == before {
            break;
        }
    }
}

static REFUSED: OnceLock<Vec<(String, Refusal)>> = OnceLock::new();
static INITIALIZED_REFUSALS: OnceLock<Vec<(String, Refusal)>> = OnceLock::new();

/// Report through the initialized game logger and the server owner's stdout.
/// A closed console pipe must not turn a mod refusal into a panic.
fn report_refusals(refused: &[(String, Refusal)], stdout: &mut impl std::io::Write) {
    for (id, why) in refused {
        log::warn!("wasm mod {id} refused: {why}");
        let _ = writeln!(stdout, "wasm mod {id} refused: {why}");
    }
}

/// The wasm mods refused at startup, each with its reason, for the Mods page.
pub fn refused() -> &'static [(String, Refusal)] {
    INITIALIZED_REFUSALS
        .get()
        .or_else(|| REFUSED.get())
        .map_or(&[], Vec::as_slice)
}

/// The current startup refusal or live trap/limit status, for a Mods-page entry.
pub fn refusal(id: &str) -> Option<Refusal> {
    refused()
        .iter()
        .find(|(owner, _)| owner == id)
        .map(|(_, why)| why.clone())
        .or_else(|| {
            MODULES
                .get()?
                .iter()
                .find(|(manifest, _)| manifest.id == id)
                .and_then(|(_, module)| module.is_stopped().then_some(Refusal::Stopped))
        })
}

/// Loads the wasm mods selected by the actual startup arguments, once at startup.
/// `player_dir` is the client's already resolved options directory, not the shared game/cache
/// directory. Dedicated startup reads properties in its game directory instead.
/// Refusals are reported and published for the Mods page and content filtering.
pub fn load_installed(
    launch: &horizon_mod_package::selection::LaunchArguments,
    player_dir: &Path,
) -> Loaded {
    let game_dir = &launch.context.game_dir;
    let dev = &launch.context.dev_mods;
    let budget = compile_budget::CompileBudget::read(launch, player_dir);
    let mut cache = module_cache::ModuleCache::new(game_dir.join(module_cache::CACHE_DIR));
    if let Ok(budget) = budget.as_ref() {
        cache.budget = *budget;
    }
    let mod_list = horizon_mod_package::mod_list::ModList::load(game_dir);
    let memory_cap_mib = mod_list.wasm_memory_cap();
    let enabled = mod_list.enabled;
    let mut refused: Vec<_> = horizon_mod_package::selection::package_errors(
        horizon_mod_package::find(&game_dir.join(horizon_mod_package::mod_list::DIR))
            .into_iter()
            .chain(horizon_mod_package::find(
                &game_dir.join(horizon_mod_package::mod_list::NATIVE_DIR),
            ))
            .chain(dev.iter().cloned()),
    )
    .into_iter()
    .filter(|(id, _)| enabled.contains(id) || !dev.is_empty())
    .map(|(id, why)| (id, Refusal::Package(why)))
    .collect();
    let native: Vec<_> =
        horizon_mod_package::find(&game_dir.join(horizon_mod_package::mod_list::NATIVE_DIR))
            .into_iter()
            .chain(horizon_mod_package::find(
                &game_dir.join(horizon_mod_package::mod_list::DIR),
            ))
            .chain(std::env::current_exe().ok().into_iter().flat_map(|exe| {
                horizon_mod_package::find(
                    &exe.with_file_name(horizon_mod_package::mod_list::NATIVE_DIR),
                )
            }))
            .chain(dev.iter().cloned())
            .filter_map(|path| horizon_mod_package::Package::read(&path).ok())
            .filter(|package| package.is_native())
            .collect();
    let packages: Vec<WasmPackage> =
        horizon_mod_package::selection::content_packages(game_dir, &enabled, dev)
            .filter(|selected| selected.package.manifest.wasm.is_some())
            .filter_map(|selected| {
                if let Some(reason) = native.iter().find_map(|library| {
                    horizon_mod_package::layouts::mixed_refusal(&selected.package, library)
                }) {
                    refused.push((selected.package.manifest.id, Refusal::Package(reason)));
                    return None;
                }
                match horizon_mod_package::Package::read_wasm(&selected.path) {
                    Ok(module) => Some(WasmPackage {
                        manifest: selected.package.manifest,
                        module,
                    }),
                    Err(why) => {
                        refused.push((selected.package.manifest.id, Refusal::Package(why)));
                        None
                    }
                }
            })
            .collect();
    let mut loaded = match budget {
        Err(why) => Loaded {
            refused: packages
                .into_iter()
                .map(|package| (package.manifest.id, Refusal::CompileSetting(why.clone())))
                .collect(),
            ..Loaded::default()
        },
        Ok(_) => {
            let std = std::env::current_exe()
                .ok()
                .and_then(|exe| std::fs::read(exe.with_file_name("horizon-wasm-std.wasm")).ok());
            load_available_std(
                std.as_deref(),
                packages,
                Some(&cache),
                memory_cap_mib,
                launch.installation_side(),
            )
        }
    };
    refused.append(&mut loaded.refused);
    report_refusals(&refused, &mut std::io::stdout().lock());
    let _ = REFUSED.set(refused.clone());
    loaded.refused = refused;
    let _ = MODULES.set(
        loaded
            .manifests
            .iter()
            .cloned()
            .zip(loaded.modules.iter().cloned())
            .collect(),
    );
    loaded
}

/// Loaded wasm entries are installed in the same dispatch list as native entries.
#[derive(Default)]
pub struct Loaded {
    pub mods: Vec<mod_api::ModEntry>,
    pub manifests: Vec<Manifest>,
    pub refused: Vec<(String, Refusal)>,
    modules: Vec<WasmMod>,
}

impl Loaded {
    /// Initializes programmatically loaded mods with the final inventory, then installs
    /// their point listeners. Call once, before sealing the point registry.
    pub fn initialize(
        &self,
        inventory: &[mod_api::PackageInfo],
    ) -> Result<(), Vec<(String, Refusal)>> {
        let refused = initialize_selected(self.manifests.iter().zip(&self.modules), inventory);
        if refused.is_empty() {
            Ok(())
        } else {
            Err(refused)
        }
    }
}

static MODULES: OnceLock<Vec<(Manifest, WasmMod)>> = OnceLock::new();

/// Supplies the complete native/content inventory after the game's dispatch order is installed,
/// before registration. Constructors and subscription queries run before this, as on native.
pub fn initialize(inventory: &[mod_api::PackageInfo]) {
    let mut refused = REFUSED.get().cloned().unwrap_or_default();
    let modules = MODULES.get().map_or(&[][..], Vec::as_slice);
    refused.extend(initialize_selected(
        modules.iter().map(|(manifest, module)| (manifest, module)),
        inventory,
    ));
    let _ = INITIALIZED_REFUSALS.set(refused);
}
fn initialize_selected<'a>(
    modules: impl Iterator<Item = (&'a Manifest, &'a WasmMod)>,
    inventory: &[mod_api::PackageInfo],
) -> Vec<(String, Refusal)> {
    let mut refused = Vec::new();
    for (manifest, module) in modules {
        let result = if initialize_modules(std::slice::from_ref(module), inventory) {
            points::listen(manifest, module)
        } else {
            Err(Refusal::Runtime("initialization failed".into()))
        };
        if let Err(why) = result {
            module.disable();
            refused.push((manifest.id.clone(), why));
        }
    }
    report_refusals(&refused, &mut std::io::stdout().lock());
    refused
}
fn initialize_modules(modules: &[WasmMod], inventory: &[mod_api::PackageInfo]) -> bool {
    let mut succeeded = true;
    for module in modules {
        succeeded &= module
            .call(
                CallClass::Load,
                boundary::Frame::default(),
                mod_api::wire::INIT,
                None,
                |w| w.put(inventory),
                |_| Ok(()),
            )
            .is_some();
    }
    succeeded
}

fn load_available_std(
    std: Option<&[u8]>,
    packages: Vec<WasmPackage>,
    cache: Option<&module_cache::ModuleCache>,
    memory_cap_mib: u32,
    side: horizon_mod_package::manifest::Side,
) -> Loaded {
    match std {
        Some(std) => load_with_cache(std, packages, cache, memory_cap_mib, side),
        None => Loaded {
            refused: packages
                .into_iter()
                .map(|p| (p.manifest.id, Refusal::NoStd))
                .collect(),
            ..Loaded::default()
        },
    }
}

/// Loads `packages` with wasm32's whole 4 GiB as the groups' memory cap.
pub fn load(std_member: &[u8], packages: Vec<WasmPackage>) -> Loaded {
    load_with_cache(
        std_member,
        packages,
        None,
        horizon_mod_package::manifest::MAX_WASM_MEMORY_MIB,
        horizon_mod_package::manifest::Side::Both,
    )
}

/// A group's memory, in MiB: the largest any member declares, else 1 GiB, within the instance's
/// cap. A declaration above the cap refuses the group, naming it and both numbers.
fn group_memory(manifests: &[(&str, &Manifest)], cap_mib: u32) -> Result<u32, Refusal> {
    match manifests
        .iter()
        .filter_map(|(_, manifest)| manifest.wasm_memory_mib)
        .max()
    {
        Some(declared_mib) if declared_mib > cap_mib => Err(Refusal::Memory {
            group: manifests
                .iter()
                .map(|(_, manifest)| manifest.id.as_str())
                .collect::<Vec<_>>()
                .join("+"),
            declared_mib,
            cap_mib,
        }),
        Some(declared_mib) => Ok(declared_mib),
        None => Ok(runtime::DEFAULT_GROUP_MEMORY_MIB.min(cap_mib)),
    }
}

fn load_with_cache(
    std_member: &[u8],
    packages: Vec<WasmPackage>,
    cache: Option<&module_cache::ModuleCache>,
    memory_cap_mib: u32,
    side: horizon_mod_package::manifest::Side,
) -> Loaded {
    let (groups, refused) = groups(&packages, side);
    let mut loaded = Loaded {
        refused,
        ..Loaded::default()
    };
    let engine = match runtime::engine() {
        Ok(engine) => engine,
        Err(error) => {
            loaded.refused.extend(
                packages
                    .iter()
                    .map(|p| (p.manifest.id.clone(), Refusal::Runtime(error.to_string()))),
            );
            return loaded;
        }
    };
    for indices in groups {
        let mut members = vec![link::Member {
            name: "libstd.so",
            module: std_member,
        }];
        let names: Vec<_> = indices
            .iter()
            .map(|&i| format!("lib{}.so", packages[i].manifest.id))
            .collect();
        members.extend(indices.iter().zip(&names).map(|(&i, name)| link::Member {
            name,
            module: &packages[i].module,
        }));
        let manifests: Vec<_> = indices
            .iter()
            .zip(&names)
            .map(|(&i, name)| (name.as_str(), &packages[i].manifest))
            .collect();
        let result = group_memory(&manifests, memory_cap_mib)
            .and_then(|memory_mib| WasmMod::instantiate(&engine, memory_mib, &members, cache))
            .and_then(|mods| {
                if mods.len() != indices.len() {
                    return Err(Refusal::Runtime("missing mod boundary exports".into()));
                }
                let mut checked = Vec::new();
                for mut module in mods {
                    let subscriptions: Vec<(String, i32, bool)> = module
                        .call(
                            CallClass::Load,
                            boundary::Frame::default(),
                            boundary::SUBSCRIBED,
                            None,
                            |_| {},
                            |r| r.take(),
                        )
                        .ok_or_else(|| Refusal::Runtime("subscriptions failed".into()))?;
                    registration_capabilities::check(&subscriptions)?;
                    module
                        .check_prepared_hooks(
                            &subscriptions
                                .iter()
                                .map(|(name, _, _)| boundary::hook_id(name))
                                .collect::<Vec<_>>(),
                        )
                        .map_err(Refusal::Runtime)?;
                    let names: Vec<String> = subscriptions
                        .iter()
                        .map(|(name, _, _)| name.clone())
                        .collect();
                    module.listeners = subscriptions
                        .into_iter()
                        .filter_map(|(name, priority, receive_cancelled)| {
                            boundary::hook_id(&name).map(|hook| mod_api::Listener {
                                hook: Some(hook),
                                priority,
                                receive_cancelled,
                            })
                        })
                        .collect::<Vec<_>>()
                        .leak();
                    // The ground provider runs on the worldgen workers, a contract wasm lacks yet.
                    if names.iter().any(|name| name == "alter_ground") {
                        return Err(Refusal::Runtime(
                        "alter_ground: worldgen ground providers are not yet available to wasm mods"
                            .into(),
                    ));
                    }
                    // A hook the wasm side cannot serve yet refuses the load by
                    // name with its reason, never a silent skip past a dead
                    // subscription.
                    if let Some(reason) = names.iter().find_map(|name| native_only_hook(name)) {
                        return Err(Refusal::Runtime(reason.into()));
                    }
                    module.subscribed = boundary::subscriptions_of("wasm mod", names);
                    checked.push(module);
                }
                Ok(checked)
            });
        match result {
            Ok(mods) => {
                for (module, i) in mods.into_iter().zip(indices) {
                    let manifest = packages[i].manifest.clone();
                    let id = Box::leak(manifest.id.clone().into_boxed_str());
                    loaded.modules.push(module.clone());
                    loaded
                        .mods
                        .push(mod_api::ModEntry::wasm(id, Box::new(module)));
                    loaded.manifests.push(manifest);
                }
            }
            Err(why) => loaded.refused.extend(
                indices
                    .into_iter()
                    .map(|i| (packages[i].manifest.id.clone(), why.clone())),
            ),
        }
    }
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wasm_mod_using_the_general_entity_tick_is_refused_by_name() {
        // The reason names the hook and the follow-up, as `alter_ground`'s
        // inline refusal does; nothing else the API names refuses here.
        let reason = native_only_hook("general_entity_tick").unwrap();
        assert!(reason.starts_with("general_entity_tick:"));
        assert!(reason.contains("stage 4b"));
        assert_eq!(native_only_hook("entity_tick_pre"), None);
        assert_eq!(native_only_hook("alter_ground"), None);
    }

    #[test]
    fn point_layout_refusal_reports_both_layouts_to_console() {
        let mut console = Vec::new();
        report_refusals(
            &[(
                "layout_mod".into(),
                Refusal::PointLayout {
                    built: 123,
                    game: 456,
                },
            )],
            &mut console,
        );
        assert_eq!(
            String::from_utf8(console).unwrap(),
            "wasm mod layout_mod refused: built for another version of the mod points API (mod layout 123, game layout 456)\n"
        );
    }

    #[test]
    fn missing_std_reports_mod_and_reason_to_console_and_logger() {
        use crate::test_log::{CAPTURE, capture_logs};
        capture_logs();
        let mut console = Vec::new();
        let loaded = load_available_std(
            None,
            vec![package("missing_std_mod", "1.0.0", "", 'a', "t", "")],
            None,
            horizon_mod_package::manifest::MAX_WASM_MEMORY_MIB,
            horizon_mod_package::manifest::Side::Both,
        );
        assert!(loaded.mods.is_empty());
        assert!(matches!(loaded.refused[0].1, Refusal::NoStd));
        report_refusals(&loaded.refused, &mut console);
        let expected =
            "wasm mod missing_std_mod refused: this game has no wasm standard library member";
        assert_eq!(String::from_utf8(console).unwrap(), format!("{expected}\n"));
        assert!(
            CAPTURE
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|line| line == expected)
        );
    }

    #[test]
    fn installed_wasm_mixin_refuses_by_name() {
        let root = std::env::temp_dir().join(format!("wasm-mixin-{}", std::process::id()));
        let package = root.join("bad_mixin");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("horizon-mod.toml"), format!(
            "id = 'bad_mixin'\nname = 'Bad mixin'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[wasm]\nsha512 = '{}'\ntoolchain = 'test'\nbuilt_with = 'test'\n[[native.mixin]]\nid = 'patch'\ntarget = 'game::tick'\nkind = 'HEAD'\nhelper = 'src/patch.rs'\n", "a".repeat(128)
        )).unwrap();
        let launch = horizon_mod_package::selection::LaunchArguments::parse(
            ["--dev-mod".to_owned(), package.to_str().unwrap().to_owned()].into_iter(),
            &root,
            &root,
            None,
        )
        .unwrap();
        let loaded = load_installed(&launch, &root);
        assert!(
            loaded.refused.iter().any(|(id, reason)| id == "bad_mixin"
                && reason
                    .to_string()
                    .contains("only [native] tables compile game code")),
            "{:?}",
            loaded.refused
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_refuses_prepared_subscription_without_exports() {
        crate::test_stack::on_guest_stack(|| {
            let std =
                wat::parse_str(r#"(module (@custom "dylink.0" "\01\04\00\00\00\00"))"#).unwrap();
            let mut subscriptions = mod_api::wire::Writer::new();
            subscriptions.put(&vec![("can_equip".to_owned(), 0i32, false)]);
            let stores = subscriptions
                .bytes()
                .iter()
                .enumerate()
                .map(|(i, b)| format!("i32.const {} i32.const {b} i32.store8 ", 1024 + i))
                .collect::<String>();
            let mut package = package("missing_prepared", "1.0.0", "", 'a', "t", "");
            package.module = wat::parse_str(format!(
                r#"(module
                (@custom "dylink.0" "\01\04\00\00\00\00")
                (import "env" "memory" (memory 1))
                (func (export "horizon_layout") (result i64) i64.const {})
                (func (export "horizon_in") (param i32) (result i32) i32.const 512)
                (func (export "horizon_hook") (param i32) (result i64) {stores} i64.const {})
            )"#,
                *mod_api::wire::LAYOUT,
                (1024u64 << 32) | subscriptions.bytes().len() as u64
            ))
            .unwrap();
            let loaded = load(&std, vec![package]);
            assert!(loaded.mods.is_empty());
            assert_eq!(loaded.refused.len(), 1);
            let reason = loaded.refused[0].1.to_string();
            assert!(
                reason.contains("can_equip") && reason.contains("prepare_can_equip"),
                "{reason}"
            );
        });
    }

    fn package(
        id: &str,
        version: &str,
        deps: &str,
        sha: char,
        toolchain: &str,
        linked: &str,
    ) -> WasmPackage {
        let text = format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n\n[dependencies]\n{deps}\n[wasm]\nsha512 = \"{}\"\ntoolchain = \"{toolchain}\"\nbuilt_with = \"test\"\n{linked}",
            sha.to_string().repeat(128)
        );
        WasmPackage {
            manifest: Manifest::parse(&text).unwrap(),
            module: Vec::new(),
        }
    }

    fn declaring(id: &str, sha: char, memory_mib: Option<u32>) -> WasmPackage {
        let mut package = package(id, "1.0.0", "", sha, "t", "");
        package.manifest.wasm_memory_mib = memory_mib;
        package
    }

    #[test]
    fn a_group_takes_its_largest_declared_memory_within_the_instance_cap() {
        let library = declaring("big_lib", 'a', Some(2048));
        let addon = declaring("addon", 'b', Some(1536));
        let plain = declaring("plain", 'c', None);
        let group = |members: &[&WasmPackage]| -> Result<u32, Refusal> {
            let manifests: Vec<_> = members.iter().map(|p| ("member", &p.manifest)).collect();
            group_memory(&manifests, 4096)
        };
        assert_eq!(group(&[&plain]).unwrap(), 1024);
        assert_eq!(group(&[&library, &addon, &plain]).unwrap(), 2048);
        assert_eq!(
            group_memory(&[("member", &plain.manifest)], 512).unwrap(),
            512
        );
    }

    #[test]
    fn a_declared_need_above_the_instance_cap_refuses_the_group_at_load_by_name() {
        let loaded = load_with_cache(
            b"",
            vec![
                declaring("big_lib", 'a', Some(2048)),
                package("addon", "1.0.0", "big_lib = \"1\"\n", 'b', "t", ""),
            ],
            None,
            1024,
            horizon_mod_package::manifest::Side::Both,
        );
        assert!(loaded.mods.is_empty());
        let mut refused: Vec<_> = loaded
            .refused
            .iter()
            .map(|(id, why)| format!("{id}: {why}"))
            .collect();
        refused.sort();
        let why = "wasm group big_lib+addon declares 2048 MiB of memory (wasm_memory_mib), above this instance's cap of 1024 MiB (wasm.memory_cap_mib in horizon-mods.txt)";
        assert_eq!(
            refused,
            [format!("addon: {why}"), format!("big_lib: {why}")]
        );
    }

    fn ids(packages: &[WasmPackage], groups: &[Vec<usize>]) -> Vec<Vec<String>> {
        groups
            .iter()
            .map(|g| g.iter().map(|&i| packages[i].manifest.id.clone()).collect())
            .collect()
    }

    #[test]
    fn mods_sharing_a_library_share_one_group_in_dependency_order() {
        let packages = vec![
            package("b", "1.0.0", "lib = \"^1\"", 'b', "t", ""),
            package("lib", "1.2.0", "", 'a', "t", ""),
            package("c", "1.0.0", "lib = \"^1\"", 'c', "t", ""),
            package("alone", "1.0.0", "", 'd', "t", ""),
        ];
        let (groups, refused) = groups(&packages, horizon_mod_package::manifest::Side::Both);
        assert!(refused.is_empty());
        assert_eq!(
            ids(&packages, &groups),
            [vec!["lib", "b", "c"], vec!["alone"]]
        );
    }

    #[test]
    fn bundle_ties_and_cycle_refusals_match_the_native_resolver() {
        let packages = vec![
            package("z", "1", "lib = '^1'", 'b', "t", ""),
            package("lib", "1.2", "", 'a', "t", ""),
            package("a", "1", "lib = '^1'", 'c', "t", ""),
        ];
        let (group, refused) = groups(&packages, horizon_mod_package::manifest::Side::Both);
        assert!(refused.is_empty());
        assert_eq!(ids(&packages, &group), [vec!["lib", "z", "a"]]);
        let declared: Vec<_> = packages
            .iter()
            .map(|p| {
                dependencies::from_manifest(
                    &p.manifest.dependencies,
                    horizon_mod_package::manifest::Side::Both,
                )
            })
            .collect();
        let native: Vec<_> = packages
            .iter()
            .zip(&declared)
            .map(|(p, dependencies)| dependencies::Mod {
                id: &p.manifest.id,
                version: &p.manifest.version,
                has_api: true,
                code: true,
                dependencies,
            })
            .collect();
        assert_eq!(group[0], dependencies::resolve(&native).unwrap().order);
        let cyclic = vec![
            package("a", "1", "b = '^1'", 'a', "t", ""),
            package("b", "1", "a = '^1'", 'b', "t", ""),
            package("user", "1", "a = '^1'", 'c', "t", ""),
            package("free", "1", "", 'd', "t", ""),
        ];
        let (group, refused) = groups(&cyclic, horizon_mod_package::manifest::Side::Both);
        assert_eq!(ids(&cyclic, &group), [vec!["free"]]);
        assert!(
            matches!(&refused[0].1, Refusal::DependencyOrder { reason } if reason.contains("a -> b -> a"))
        );
        assert!(matches!(&refused[1].1, Refusal::DependencyOrder { .. }));
        assert!(matches!(&refused[2].1, Refusal::DependencyRefused { .. }));
    }

    #[test]
    fn a_missing_required_dependency_refuses_the_mod_and_its_dependents() {
        let packages = vec![
            package("lib", "2.0.0", "", 'a', "t", ""),
            package("b", "1.0.0", "missing = \"^1\"", 'b', "t", ""),
            package(
                "c",
                "1.0.0",
                "b = \"^1\"\nextra = { version = \"^1\", optional = true }",
                'c',
                "t",
                "",
            ),
        ];
        let (groups, refused) = groups(&packages, horizon_mod_package::manifest::Side::Both);
        assert_eq!(ids(&packages, &groups), [vec!["lib"]]);
        assert_eq!(
            refused,
            [
                (
                    "b".to_string(),
                    Refusal::MissingDependency {
                        dependency: "missing".into(),
                        version: "^1".into()
                    }
                ),
                (
                    "c".to_string(),
                    Refusal::DependencyRefused {
                        dependency: "b".into()
                    }
                ),
            ]
        );
    }

    #[test]
    fn declared_internals_refuse_the_mod_and_its_dependents_but_an_injection_loads() {
        let inject = "[[inject]]\nid = \"stop\"\nat = \"horizon_client::game::server_lifecycle::remember_shutdown\"\nkind = \"head\"\nwith = \"on_stop\"\nhash = \"h\"\n";
        let packages = vec![
            package(
                "deep",
                "1.0.0",
                "",
                'a',
                "t",
                "[[internals]]\npath = \"Game::world\"\nkind = \"field\"\nhash = \"h\"\n",
            ),
            package("user", "1.0.0", "deep = \"^1\"", 'b', "t", ""),
            package("typed", "1.0.0", "", 'c', "t", inject),
        ];
        let (groups, refused) = groups(&packages, horizon_mod_package::manifest::Side::Both);
        assert_eq!(ids(&packages, &groups), [vec!["typed"]]);
        assert_eq!(
            refused,
            [
                ("deep".to_string(), Refusal::Internals),
                (
                    "user".to_string(),
                    Refusal::DependencyRefused {
                        dependency: "deep".into()
                    }
                ),
            ]
        );
        assert_eq!(
            Refusal::Internals.to_string(),
            "internals access is native-only"
        );
    }

    #[test]
    fn another_build_of_a_dependency_or_toolchain_is_refused() {
        let linked = format!("[wasm.linked]\nlib = \"{}\"\n", "f".repeat(128));
        let packages = vec![
            package("lib", "1.0.0", "", 'a', "t", ""),
            package("b", "1.0.0", "lib = \"^1\"", 'b', "t", &linked),
            package("c", "1.0.0", "lib = \"^1\"", 'c', "other", ""),
        ];
        let (_, refused) = groups(&packages, horizon_mod_package::manifest::Side::Both);
        assert_eq!(
            refused,
            [
                (
                    "b".to_string(),
                    Refusal::LinkedAgainst {
                        dependency: "lib".into()
                    }
                ),
                (
                    "c".to_string(),
                    Refusal::Toolchain {
                        found: "other".into(),
                        group: "t".into()
                    }
                ),
            ]
        );
    }
}

#[cfg(test)]
mod mixed_install_tests {
    use super::*;
    #[test]
    fn malformed_layout_retains_identity() {
        let why = refusal("malformed");
        assert!(why.contains("layouts.json is malformed"), "{why}");
    }
    fn refusal(hash: &str) -> String {
        let root = tempfile::tempdir().unwrap().keep();
        let addon = root.join("addon");
        let library = root.join("lib");
        std::fs::create_dir_all(&addon).unwrap();
        std::fs::create_dir_all(library.join("src")).unwrap();
        let header = |id| {
            format!("id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n")
        };
        let module = b"\x00\x61\x73\x6d\x01\x00\x00\x00\x00\x0f\x08\x64\x79\x6c\x69\x6e\x6b\x2e\x30\x01\x04\x00\x00\x00\x00\x01\x04\x01\x60\x00\x00\x03\x02\x01\x00\x07\x2e\x03\x0e\x68\x6f\x72\x69\x7a\x6f\x6e\x5f\x6c\x61\x79\x6f\x75\x74\x00\x00\x0a\x68\x6f\x72\x69\x7a\x6f\x6e\x5f\x69\x6e\x00\x00\x0c\x68\x6f\x72\x69\x7a\x6f\x6e\x5f\x68\x6f\x6f\x6b\x00\x00\x0a\x04\x01\x02\x00\x0b";
        std::fs::write(addon.join("mod.wasm"), module).unwrap();
        std::fs::write(
            addon.join("horizon-mod.toml"),
            format!(
                "{}[dependencies]\nlib='*'\n[wasm]\nsha512='{}'\ntoolchain='t'\nbuilt_with='t'\n",
                header("addon"),
                horizon_mod_package::manifest::sha512_of(module)
            ),
        )
        .unwrap();
        std::fs::write(
            library.join("horizon-mod.toml"),
            format!(
                "{}[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
                header("lib"),
                "a".repeat(40),
                "b".repeat(40)
            ),
        )
        .unwrap();
        std::fs::write(
            library.join("Cargo.toml"),
            "[package]\nname='lib'\nversion='1.0.0'\n",
        )
        .unwrap();
        std::fs::write(library.join("src/lib.rs"), "pub fn run() {}\n").unwrap();
        std::fs::write(
            addon.join("layouts.json"),
            br#"{"lib":{"T":"0123456789abcdef"}}"#,
        )
        .unwrap();
        std::fs::write(
            library.join("layouts.json"),
            format!(r#"{{"lib":{{"T":"{hash}"}}}}"#),
        )
        .unwrap();
        if hash == "malformed" {
            std::fs::write(addon.join("layouts.json"), b"{").unwrap();
            std::fs::write(library.join("layouts.json"), b"{}").unwrap();
        }
        let launch = horizon_mod_package::selection::LaunchArguments::parse(
            [
                "--dev-mod".to_owned(),
                addon.to_str().unwrap().to_owned(),
                "--dev-mod".to_owned(),
                library.to_str().unwrap().to_owned(),
            ]
            .into_iter(),
            &root,
            &root,
            None,
        )
        .unwrap();
        let loaded = load_installed(&launch, &root);
        std::fs::remove_dir_all(root).unwrap();
        assert!(loaded.mods.is_empty());
        assert_eq!(loaded.refused.len(), 1);
        assert_eq!(loaded.refused[0].0, "addon");
        loaded.refused[0].1.to_string()
    }
    #[test]
    fn wasm_to_native_layout_mismatch_names_the_type() {
        assert_eq!(
            refusal("1123456789abcdef"),
            "addon: layout mismatch in library lib for type T"
        );
    }
    #[test]
    fn wasm_to_native_matching_layout_is_still_unsupported() {
        assert_eq!(
            refusal("0123456789abcdef"),
            "lib is a native mod; wasm mod addon depends on it: mixed native and wasm dependencies aren't supported yet"
        );
    }
}

/// Resolve generated boundary operations for hand-written guest integration fixtures.
#[cfg(feature = "listener-fixture")]
pub fn fixture_import_id(owner: &str, method: &str) -> Option<u32> {
    boundary::import_id(owner, method)
}
/// Resolve generated hook operations for hand-written guest integration fixtures.
#[cfg(feature = "listener-fixture")]
pub fn fixture_export_id(owner: &str, method: &str) -> Option<u32> {
    boundary::export_id(owner, method)
}

/// The private compiler cache key under the production engine configuration, for executable
/// smoke tests of the hidden compiler entry.
#[cfg(feature = "runtime-fixture")]
pub fn compile_child_key(bytes: &[u8]) -> Result<String, String> {
    let engine = runtime::engine().map_err(|error| error.to_string())?;
    Ok(module_cache::ModuleCache::key(&engine, bytes))
}
