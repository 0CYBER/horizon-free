//! The WebAssembly interface of the mod API: every method of the traits a mod calls (the hosts,
//! the backends, the drawing contexts) and every method it implements (`Hooks`, the parts, the
//! capability handlers), through one shared-memory boundary and the glue on both sides of it.
mod append;
mod borrowed;
mod calls;
mod host;
mod prepared;
#[cfg(test)]
mod prepared_tests;
pub use host::{generate_coverage, generate_host};
mod trait_roles;
mod ty;

use crate::capability_providers::key_shapes;
use crate::index::{Found, Index};
use std::fmt::Write as _;
use std::path::Path;
use syn::{FnArg, Item, Pat, ReturnType, TraitItem};
use ty::{Ty, Types};

/// Traits a mod calls: their methods are the world's imports.
pub(super) const IMPORTS: &[&str] = &[
    "Host",
    "ServerHost",
    "ClientHost",
    "Backend",
    "BackendMut",
    "ClientBackendMut",
    "DrawContext",
    "BlockEntityDrawContext",
    "WorldDrawContext",
    "CameraDrawContext",
    "LevelHost",
    "LevelRandomHost",
    "LevelTicksHost",
    "LevelWriteHost",
    "ConfigFiles",
    "Commands",
    "Levels",
    "Messages",
    "Players",
    "RuntimeLevels",
    "ClientSession",
    "RenderAssets",
    "InstanceGroups",
    "Tags",
    "item::menu::SlotView",
    "Registrar",
    "CapabilityProviders",
    "TicketValidation",
    "DataLoadContext",
    "LoopContext",
    "WorldgenHost",
    "GenerationContext",
    "PieceContext",
    "StructureRandom",
    "AttachmentStore",
    "ServerReloadListeners",
    "GameTests",
    "ClientReloadListeners",
    "PackFinders",
    "ColorResolvers",
    "BlockTintSources",
    "CauldronInteractions",
    "SectionGeometryRenderers",
    "SectionGeometryContext",
    "ParticleProviders",
    "EnchantmentDamageEffects",
    "PoiTypeExtensions",
    "VillagerProfessions",
    "LoadConditions",
    "CauldronFluidContents",
    "BlockAndTintGetter",
    "GroundLevel",
    "BlockMetadata",
    "ServerReloadResources",
    "CriterionContext",
    "UnspawnedMobEdits",
];

/// Imported traits a call never hands over, each reached through another's method instead.
pub(super) const REACHED: &[(&str, &str, &str)] = &[
    ("ConfigFiles", "", ""),
    ("Commands", "server_host_mut", "Ok(h.commands())"),
    ("Levels", "server_host", "Ok(h.levels())"),
    ("Messages", "server_host", "Ok(h.messages())"),
    ("Players", "server_host", "Ok(h.players())"),
    ("RuntimeLevels", "server_host_mut", "Ok(h.runtime_levels())"),
    ("ClientSession", "client_host", "Ok(h.client_session())"),
    ("RenderAssets", "client_host", "Ok(h.render_assets())"),
    ("InstanceGroups", "client_host", "Ok(h.instance_groups())"),
    // A level hook's host first: inside a level write it is the one the call hands over.
    (
        "AttachmentStore",
        "",
        "level_random_host_mut().map(|h| h.attachments()).or_else(|_| host_mut().map(|h| h.attachments()))",
    ),
    (
        "CapabilityProviders",
        "host",
        "Ok(h.capability_providers())",
    ),
    (
        "ServerReloadResources",
        "server_host",
        "Ok(h.reload_resources())",
    ),
    (
        "Backend",
        "host",
        "h.backend().map(crate::BackendRef::open).ok_or_else(|| missing(\"game backend\"))",
    ),
    (
        "BackendMut",
        "server_host_mut",
        "crate::backend_mut_of(h).ok_or_else(|| missing(\"game backend\"))",
    ),
    (
        "ClientBackendMut",
        "client_host_mut",
        "crate::client_backend_mut_of(h).ok_or_else(|| missing(\"client game backend\"))",
    ),
];

/// Bounded file-capability selectors, shared by the guest proxy and host dispatch.
pub(super) const FILE_ROOTS: &[(u8, &str, &str)] = &[
    (0, "Host", "config_files"),
    (1, "Host", "data_files"),
    (2, "ServerHost", "world_data_files"),
];

/// Traits a mod implements: their methods are the world's exports. The handler traits the
/// capability keys name join them (`exports`).
const EXPORTS: &[&str] = &[
    "Hooks",
    "ReloadApply",
    "BlockEntityTransform",
    "StructureType",
    "StructurePiece",
    "BlockPart",
    "BlockEntityPart",
    "ItemPart",
    "MenuPart",
    "FluidPart",
    "ServerReloadListener",
    "LoadCondition",
    "GameTestFunction",
    "ClientReloadListener",
    "BiomeColorResolver",
    "BlockTintSource",
    "Interaction",
    "SectionGeometry",
    "ParticleProvider",
    "EnchantmentDamageEffect",
    "CommandArgumentType",
    "EntitySelectorType",
    "GroundStateProvider",
    "PlaceProvider",
    "TooltipAppender",
    "CriterionTest",
];

/// Every exported trait: [`EXPORTS`], then the handler traits `mod_api`'s capability keys name
/// (`ResourceHandler<ItemResource>`), each instance of a generic one its own export. Set once,
/// by `surface`.
pub(super) fn exports() -> &'static [&'static str] {
    trait_roles::exports()
}

/// All imported traits, including discovered per-item declarations.
pub(super) fn imports() -> &'static [&'static str] {
    trait_roles::imports()
}

/// Whether the game also hands a mod objects of exported trait `owner` (a capability's handler,
/// another mod's provider): the mod calls one through the `held-<interface>` imports, by the
/// index the game keeps it under.
pub(super) fn held(owner: &str) -> bool {
    owner == "PlaceProvider" || (exports().contains(&owner) && !EXPORTS.contains(&owner))
}

/// The contexts of the load phase: the catalog's registration and data-load contexts and a
/// resource reload's. A call given one reads and parses data, so it runs within the load limit
/// (`Budgets::load`), not a hook call's.
const LOAD_CONTEXTS: &[&str] = &[
    "RegistrationContext",
    "DataLoadContext",
    "ServerReloadContext",
    "ClientReloadContext",
];

/// The traits the game calls on its worker threads: a mod's structure types and their pieces on
/// the worldgen workers, its block tint sources, colour resolvers and section geometry on the
/// meshing workers. They keep no mutable state and get no `Host`.
const WORKERS: &[&str] = &[
    "BlockEntityTransform",
    "StructureType",
    "StructurePiece",
    "BlockTintSource",
    "BiomeColorResolver",
    "SectionGeometry",
];

/// What a call of `method` is to the runtime, for its limits and its wait. A method given the
/// worldgen context (`alter_ground`, a worldgen Part method) runs on the worldgen workers too.
pub(super) fn call_class(types: &ty::Types<'_>, method: &Method) -> &'static str {
    if WORKERS.contains(&method.owner) || takes(types, method, &["WorldgenHost"]) {
        "super::Call::Worker"
    } else if takes(types, method, LOAD_CONTEXTS) {
        "super::Call::Load"
    } else {
        "super::Call::Hook"
    }
}

/// Whether one of `method`'s arguments is one of `contexts`.
fn takes(types: &ty::Types<'_>, method: &Method, contexts: &[&str]) -> bool {
    method.params.iter().any(|(_, ty)| {
        let ty = match ty {
            ty::Ty::Ref(inner, _) => &**inner,
            ty => ty,
        };
        match ty {
            ty::Ty::Dyn(name, _) => contexts.contains(&name.as_str()),
            ty::Ty::Named(def) => contexts
                .iter()
                .any(|name| types.defs[*def].rust.rsplit("::").next() == Some(*name)),
            _ => false,
        }
    })
}

/// A trait object that crosses without methods: a provider of a mod's own handler type,
/// erased (`Arc<dyn Any + Send + Sync>`), which only mods of its group take back.
pub(super) const ERASED: &str = "Any";

/// An owner as written (`ResourceHandler<ItemResource>`): the trait's path, and its type arguments.
fn owner_parts(owner: &str) -> Result<(Vec<String>, Vec<syn::Type>), String> {
    let path: syn::Path = syn::parse_str(owner).map_err(|e| format!("{owner}: {e}"))?;
    let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    let args = match &path.segments.last().unwrap().arguments {
        syn::PathArguments::AngleBracketed(args) => args
            .args
            .iter()
            .filter_map(|arg| match arg {
                syn::GenericArgument::Type(ty) => Some(ty.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok((segments, args))
}

/// The exported traits the mod itself answers, with no handler index, and the guest's
/// expression for its implementation of each; every other export is a registered handler's.
pub(super) const MOD_EXPORTS: &[(&str, &str)] = &[("Hooks", "super::hooks()")];

/// The guest's receiver of `owner`'s exports, if the mod itself answers them.
pub(super) fn mod_receiver(owner: &str) -> Option<&'static str> {
    MOD_EXPORTS
        .iter()
        .find(|(o, _)| *o == owner)
        .map(|(_, r)| *r)
}

/// The exported traits that list the hooks they subscribe to, and the method listing them.
pub(super) const SUBSCRIBED: &[(&str, &str)] = &[
    ("Hooks", "subscribed"),
    ("BlockPart", "block_subscribed"),
    ("BlockEntityPart", "block_entity_subscribed"),
    ("ItemPart", "item_subscribed"),
    ("MenuPart", "menu_subscribed"),
    ("FluidPart", "fluid_subscribed"),
];

/// One trait method as it crosses.
#[derive(Debug)]
pub(super) struct Method {
    /// The interface the method belongs to: the union trait a host's area traits are parts of.
    pub(super) owner: &'static str,
    /// The area trait that declares it, when it is a part of `owner` (`host::items::ItemsHost`).
    pub(super) via: Option<String>,
    pub(super) name: String,
    pub(super) access: Option<String>,
    /// `Some(mutable)` for a `self` receiver.
    pub(super) receiver: Option<bool>,
    pub(super) params: Vec<(String, Ty)>,
    pub(super) ret: Ty,
}

/// A method that can't cross, and why.
#[derive(Debug)]
pub(super) struct Unbound {
    pub(super) owner: &'static str,
    pub(super) name: String,
    pub(super) why: String,
}

pub(super) struct Surface<'a> {
    /// The methods that mention a type that does not cross: the traits still need them, and
    /// each answers that it is not available.
    pub(super) dropped: Vec<Method>,
    /// The variants of the generated `HookId`, each with its hook's name.
    pub(super) hooks: Vec<(String, String)>,
    pub(super) types: Types<'a>,
    pub(super) methods: Vec<Method>,
    pub(super) unbound: Vec<Unbound>,
    /// The methods whose doc says `Not on wasm: <why>.`
    pub(super) excused: Vec<(&'static str, String)>,
    /// Each imported trait with the area traits that are parts of it.
    pub(super) parts: Vec<(&'static str, String)>,
}

/// The line on a method's doc that keeps it off wasm, and says why.
const NOT_ON_WASM: &str = "Not on wasm: ";

/// Whether `attrs`' doc has a [`NOT_ON_WASM`] line with a reason.
fn not_on_wasm(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        syn::Meta::NameValue(doc) if doc.path.is_ident("doc") => match &doc.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(line),
                ..
            }) => line
                .value()
                .trim()
                .strip_prefix(NOT_ON_WASM)
                .is_some_and(|why| why.len() > 1),
            _ => false,
        },
        _ => false,
    })
}

/// The methods of the bound traits, with their types resolved. `hooks` and `host` are the
/// generated sources `mod_api` includes.
pub(super) fn surface<'a>(
    root: &'a Path,
    included: &'a [(&'a str, &'a str)],
) -> Result<Surface<'a>, String> {
    surface_reading(root, included, None)
}

/// [`surface`]; with `missing`, a listed boundary trait the tree does not have is pushed there
/// instead of refusing the tree (the surface export reads older trees whose API predates a
/// listed trait; the build never does).
pub(super) fn surface_reading<'a>(
    root: &'a Path,
    included: &'a [(&'a str, &'a str)],
    mut missing: Option<&mut Vec<String>>,
) -> Result<Surface<'a>, String> {
    let mut types = Types::new(Index::new(root, included))?;
    let api = types.index.module("mod_api", &[])?;
    let (imported, mut exported, paths) = trait_roles::declared(root)?;
    types.index.read.borrow_mut().extend(paths);
    for shape in key_shapes(&mut types.index)? {
        for owner in [shape.handler.clone(), shape.provider()] {
            if !exported.contains(&owner) {
                exported.push(owner);
            }
        }
    }
    trait_roles::select(imported, exported);
    let mut methods = Vec::new();
    let mut unbound = Vec::new();
    let mut excused = Vec::new();
    let mut parts = Vec::new();
    let hooks = hook_ids(&mut types)?;
    for owner in imports().iter().chain(exports()) {
        let (path, args) = owner_parts(owner)?;
        let id = match (types.index.resolve_path(api, &path), missing.as_deref_mut()) {
            (Ok(Found::Item(id)), _) => id,
            (_, Some(missing)) => {
                missing.push((*owner).to_owned());
                continue;
            }
            (Err(error), None) => return Err(error),
            (Ok(_), None) => return Err(format!("{owner} is not a mod_api trait")),
        };
        let Item::Trait(item) = types.index.item(id).clone() else {
            return Err(format!("{owner} is not a trait"));
        };
        // A generic trait's instance: its parameters as the owner's arguments.
        let mut subst = Vec::new();
        for (param, arg) in item.generics.type_params().zip(&args) {
            subst.push((param.ident.to_string(), types.ty(api, arg, &[])?));
        }
        // The trait's own methods, then those of the area traits it is a union of.
        let mut members: Vec<(usize, Option<String>, syn::TraitItemFn)> = Vec::new();
        for item in &item.items {
            if let TraitItem::Fn(function) = item {
                members.push((id.module, None, function.clone()));
            }
        }
        for bound in &item.supertraits {
            let syn::TypeParamBound::Trait(bound) = bound else {
                continue;
            };
            let segments: Vec<String> = bound
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            let area_union = (segments.len() == 4 && segments[..2] == ["crate", "host"])
                || (segments.len() == 3 && segments[..2] == ["crate", "parts"]);
            if !area_union {
                continue;
            }
            let via = segments[1..].join("::");
            let Found::Item(part) = types.index.resolve_path(api, &segments[1..])? else {
                return Err(format!("{via} is not a mod_api trait"));
            };
            let Item::Trait(part_item) = types.index.item(part).clone() else {
                return Err(format!("{via} is not a trait"));
            };
            parts.push((*owner, via.clone()));
            for item in part_item.items {
                if let TraitItem::Fn(function) = item {
                    members.push((part.module, Some(via.clone()), function));
                }
            }
        }
        for (module, via, function) in &members {
            let name = function.sig.ident.to_string();
            let subscription = SUBSCRIBED.iter().any(|(o, n)| o == owner && *n == name);
            if hidden(&function.attrs) && !subscription {
                continue;
            }
            if not_on_wasm(&function.attrs) {
                excused.push((*owner, name.clone()));
            }
            match method(&mut types, *module, owner, function, &subst) {
                Ok(mut method) => {
                    method.via = via.clone();
                    methods.push(method);
                }
                Err(why) => unbound.push(Unbound { owner, name, why }),
            }
        }
    }
    // A method the API keeps off wasm (`Not on wasm:`) is gated out of the trait on that target:
    // no table holds it, and a native build of the guest answers that it is unavailable.
    let (excused_methods, methods): (Vec<Method>, Vec<Method>) = methods
        .into_iter()
        .partition(|m| excused.iter().any(|(o, n)| *o == m.owner && *n == m.name));
    let target = root.join("target");
    std::fs::create_dir_all(&target).map_err(|e| format!("creating wire report directory: {e}"))?;
    let dropped = excused_methods;
    // Track the source of checked codecs as a build-script dependency. Module
    // lookup loads it into Index even though call generation never parses its fields.
    types
        .index
        .module("mod_api", &["wire".into(), "wire_types".into()])?;
    for owner in exports() {
        let (path, _) = owner_parts(owner)?;
        // Missing only where `missing` let the first pass skip it.
        let Ok(Found::Item(id)) = types.index.resolve_path(api, &path) else {
            continue;
        };
        let Item::Trait(item) = types.index.item(id) else {
            continue;
        };
        let mut list = vec![owner.to_string()];
        for bound in &item.supertraits {
            if let syn::TypeParamBound::Trait(t) = bound {
                let name = t.path.segments.last().unwrap().ident.to_string();
                if exports().contains(&name.as_str()) {
                    list.push(name);
                }
            }
        }
        types.views.push((owner.to_string(), list));
    }
    Ok(Surface {
        dropped,
        hooks,
        types,
        methods,
        unbound,
        excused,
        parts,
    })
}

/// A frame resource already reborrowed through `REACHED`: acquiring it on the
/// guest needs no wire call. Its trait methods are the actual imports.
pub(super) fn frame_accessor(method: &Method) -> bool {
    method.params.is_empty()
        && imports().contains(&method.owner)
        && matches!(&method.ret, Ty::Dyn(name, _) if imports().iter().any(|owner| owner.rsplit("::").next() == Some(name.as_str())))
}

/// A fallible borrowed frame getter sends only its availability result across
/// Wire. The actual borrowed host remains in the synchronous native call frame.
pub(super) fn fallible_frame_accessor(method: &Method) -> Option<(&str, bool, &Ty)> {
    if !method.params.is_empty() || !imports().contains(&method.owner) {
        return None;
    }
    let Ty::Result(ok, error) = &method.ret else {
        return None;
    };
    let Ty::Dyn(name, mutable) = &**ok else {
        return None;
    };
    imports()
        .iter()
        .any(|owner| owner.rsplit("::").next() == Some(name.as_str()))
        .then_some((name.as_str(), *mutable, &**error))
}

/// The coverage check: every method of the bound traits crosses to wasm, or its doc says why
/// not (`/// Not on wasm: <why>.`), and no method that crosses says it doesn't. The game's and
/// the mods' builds both run it, so the wasm side can't fall behind the API.
fn coverage(surface: &Surface<'_>) -> Result<(), String> {
    let excused = |owner: &str, name: &str| {
        surface
            .excused
            .iter()
            .any(|(o, n)| *o == owner && n == name)
    };
    let mut problems: Vec<String> = surface
        .unbound
        .iter()
        .filter(|u| !excused(u.owner, &u.name))
        .map(|u| {
            format!(
                "{}::{} doesn't cross to wasm ({}): fix its types, or add `/// {NOT_ON_WASM}<why>.` to its doc",
                u.owner, u.name, u.why
            )
        })
        .collect();
    problems.extend(
        surface
            .methods
            .iter()
            .filter(|m| excused(m.owner, &m.name) && !frame_accessor(m))
            .map(|m| {
                format!(
                    "{}::{} crosses to wasm: remove `{NOT_ON_WASM}` from its doc",
                    m.owner, m.name
                )
            }),
    );
    match problems.is_empty() {
        true => Ok(()),
        false => Err(problems.join("\n")),
    }
}

/// The variants of the generated `HookId`, each with its hook's name.
fn hook_ids(types: &mut Types<'_>) -> Result<Vec<(String, String)>, String> {
    let api = types.index.module("mod_api", &[])?;
    let Found::Item(id) = types.index.resolve_path(api, &["HookId".to_string()])? else {
        return Err("HookId is not a mod_api enum".into());
    };
    let Item::Enum(item) = types.index.item(id) else {
        return Err("HookId is not an enum".into());
    };
    Ok(item
        .variants
        .iter()
        .map(|v| {
            (
                v.ident.to_string(),
                heck::ToSnakeCase::to_snake_case(v.ident.to_string().as_str()),
            )
        })
        .collect())
}

fn method(
    types: &mut Types<'_>,
    module: usize,
    owner: &'static str,
    function: &syn::TraitItemFn,
    subst: &[(String, Ty)],
) -> Result<Method, String> {
    let sig = &function.sig;
    if sig.generics.type_params().next().is_some() {
        return Err("generic".into());
    }
    let mut receiver = None;
    let mut params = Vec::new();
    for input in &sig.inputs {
        match input {
            FnArg::Receiver(r) => receiver = Some(r.mutability.is_some()),
            FnArg::Typed(arg) => {
                let name = match &*arg.pat {
                    Pat::Ident(ident) => ident.ident.to_string(),
                    _ => format!("arg{}", params.len()),
                };
                let name = name.trim_start_matches('_').to_string();
                params.push((name, types.ty(module, &arg.ty, subst)?));
            }
        }
    }
    let ret = match &sig.output {
        ReturnType::Default => Ty::Unit,
        ReturnType::Type(_, ty) => types.ty(module, ty, subst)?,
    };
    Ok(Method {
        owner,
        via: None,
        name: sig.ident.to_string(),
        access: function
            .attrs
            .iter()
            .find(|a| a.path().is_ident("access"))
            .map(|a| a.parse_args::<syn::Ident>().map(|i| i.to_string()))
            .transpose()
            .map_err(|e| e.to_string())?,
        receiver,
        params,
        ret,
    })
}

fn hidden(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        matches!(&attr.meta, syn::Meta::List(list)
            if list.path.is_ident("doc") && list.tokens.to_string() == "hidden")
    })
}

/// What the boundary generates from the API's sources and its generated files: the `Wire`
/// impls and layout hash of `mod_api`, and the mod's side of every call.
pub struct Generated {
    pub wire: String,
    pub guest: String,
    /// Every source file read to derive them, for the build script to track.
    pub sources: Vec<std::path::PathBuf>,
}

pub fn generate(root: &Path, included: &[(&str, &str)]) -> Result<Generated, String> {
    let providers = crate::capability_providers::generate(root, included)?.0;
    let mut with_providers = included.to_vec();
    with_providers.push(("capability_providers_generated.rs", providers.as_str()));
    let surface = surface(root, &with_providers)?;
    coverage(&surface)?;
    let mut wire = borrowed::calls(&surface.types);
    let mut guest = calls::guest(
        &surface.types,
        &surface.methods,
        &surface.dropped,
        &surface.excused,
        &surface.parts,
    );
    guest.push_str(&calls::hook_names(&surface.hooks));
    let prepared = crate::Catalog::load(root)?
        .hooks
        .into_iter()
        .filter_map(|hook| hook.prepared.map(|field| (hook.name, field)))
        .collect::<Vec<_>>();
    guest.push_str(&calls::dispatch(&surface.types, &surface.methods, &prepared).0);
    let abis = prepared::describe(&surface.methods, &prepared)?;
    let _ = writeln!(
        wire,
        "#[doc(hidden)]\npub fn wire_core_layout() -> u64 {{ let mut hash = {:#x}; {layouts} hash }}",
        layout(&surface),
        layouts = surface.types.defs.iter().filter(|def| def.lifetimes == 0).map(|def| format!("hash = crate::wire::layout_mix(hash, <{} as crate::wire::Wire>::layout(&mut vec![]));", def.rust.replace("mod_api::", "crate::"))).collect::<String>()
    );
    writeln!(
        wire,
        "#[doc(hidden)] pub fn prepared_abi_layout() -> u64 {{ {} }}",
        prepared::layout_code(&abis)
    )
    .unwrap();
    let complete = if abis.is_empty() {
        "wire_core_layout()"
    } else {
        "crate::wire::layout_mix(wire_core_layout(),prepared_abi_layout())"
    };
    writeln!(wire, "#[doc(hidden)] pub static WIRE_LAYOUT: ::std::sync::LazyLock<u64> = ::std::sync::LazyLock::new(|| {{ {complete} }});").unwrap();
    let sources = surface.types.index.read.take();
    Ok(Generated {
        wire,
        guest,
        sources,
    })
}

/// Public owned API values available to point records, without inspecting their fields.
/// The generated record's Wire bounds check its members; Clone permits copying a borrowed
/// argument, while a live capability such as Transaction remains non-copyable.
pub(crate) fn owned_types(
    root: &Path,
    included: &[(&str, &str)],
) -> Result<std::collections::BTreeSet<String>, String> {
    let providers = crate::capability_providers::generate(root, included)?.0;
    let mut with_providers = included.to_vec();
    with_providers.push(("capability_providers_generated.rs", providers.as_str()));
    let mut surface = surface(root, &with_providers)?;
    // Every owned signature def appears in generated WIRE_LAYOUT with a Wire bound.
    // That compile-time guarantee covers macro-generated slot keys/handle codecs too;
    // no owned field traversal or second Wire-implementation discovery is needed.
    let mut clones = std::collections::HashSet::new();
    let implementations = surface
        .types
        .index
        .modules
        .iter()
        .enumerate()
        .flat_map(|(module, source)| {
            source.items.iter().filter_map(move |item| match item {
                Item::Impl(item)
                    if item.trait_.as_ref().is_some_and(|(_, path, _)| {
                        path.segments.last().is_some_and(|s| s.ident == "Clone")
                    }) =>
                {
                    Some((module, item.self_ty.clone()))
                }
                _ => None,
            })
        })
        .collect::<Vec<_>>();
    for (module, ty) in implementations {
        let syn::Type::Path(path) = *ty else {
            continue;
        };
        let path = path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>();
        if let Ok(Found::Item(id)) = surface.types.index.resolve_path(module, &path) {
            clones.insert(id);
        }
    }
    let cloned =
        |index: &Index<'_>, id| ty::derives(index.item(id), "Clone") || clones.contains(&id);
    let mut allowed = std::collections::BTreeSet::new();
    for (def, value) in surface
        .types
        .defs
        .iter()
        .enumerate()
        .filter(|(_, value)| value.lifetimes == 0)
    {
        let item = surface
            .types
            .by_item
            .iter()
            .find_map(|(item, d)| (*d == def).then_some(*item));
        if item.map_or(value.rust.starts_with("serde_json::"), |id| {
            cloned(&surface.types.index, id)
        }) {
            allowed.insert(value.rust.replace(' ', ""));
        }
    }
    // Index both the public boundary reexports and checked codec declarations.
    // These lookups load source files; they do not classify any owned fields.
    surface.types.index.module("mod_api", &["wire".into()])?;
    let codec = surface
        .types
        .index
        .module("mod_api", &["wire".into(), "wire_types".into()])?;
    let mut codecs = std::collections::HashSet::new();
    for item in surface.types.index.modules[codec].items.clone() {
        let Item::Impl(item) = item else {
            continue;
        };
        // wire_types imports precisely crate::wire::Wire. Its direct impl targets
        // include foreign reexports such as ConnectionKind, which cannot derive here.
        if !item
            .trait_
            .as_ref()
            .is_some_and(|(_, p, _)| p.segments.last().is_some_and(|s| s.ident == "Wire"))
        {
            continue;
        }
        let syn::Type::Path(path) = &*item.self_ty else {
            continue;
        };
        let path = path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>();
        if let Ok(Found::Item(id)) = surface.types.index.resolve_path(codec, &path) {
            codecs.insert(id);
        }
    }
    // Reuse the parsed module inventory. Extra declarations need an explicit
    // derive or checked codec; Clone alone (or a no-Wire reason) is never proof.
    let modules = surface
        .types
        .index
        .modules
        .iter()
        .enumerate()
        .filter(|(_, m)| m.krate == "mod_api")
        .map(|(module, m)| (module, m.path.clone(), m.items.clone()))
        .collect::<Vec<_>>();
    for (module, prefix, items) in modules {
        for item in items {
            let names = match item {
                Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    crate::capability_providers::use_names(&item.tree)
                }
                Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    vec![item.ident.to_string()]
                }
                Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                    vec![item.ident.to_string()]
                }
                _ => continue,
            };
            for name in names {
                let Ok(Found::Item(id)) = surface.types.index.resolve(module, &name) else {
                    continue;
                };
                let declaration = surface.types.index.item(id);
                let generics = match declaration {
                    Item::Struct(item) => &item.generics,
                    Item::Enum(item) => &item.generics,
                    _ => continue,
                };
                if generics.params.is_empty()
                    && (ty::derives(declaration, "Wire") || codecs.contains(&id))
                    && cloned(&surface.types.index, id)
                {
                    let path = std::iter::once("mod_api")
                        .chain(prefix.iter().map(String::as_str))
                        .chain(std::iter::once(name.as_str()))
                        .collect::<Vec<_>>()
                        .join("::");
                    if public_path(&surface.types.index, &path) {
                        allowed.insert(path);
                    }
                }
            }
        }
    }
    Ok(allowed)
}

/// A defining path is usable by generated records only through public modules.
fn public_path(index: &Index<'_>, path: &str) -> bool {
    let segments = path.split("::").skip(1).collect::<Vec<_>>();
    (0..segments.len().saturating_sub(1)).all(|depth| {
        index.modules.iter().find(|m| m.krate == "mod_api" && m.path.iter().map(String::as_str).eq(segments[..depth].iter().copied())).is_some_and(|m| m.items.iter().any(|item| matches!(item, Item::Mod(child) if child.ident == segments[depth] && matches!(child.vis, syn::Visibility::Public(_)))))
    })
}

/// A hash of everything the two sides must agree on: every method's id and signature, and
/// every type's shape (FNV-1a over their descriptions). This is the core seed;
/// `generate` appends owned Wire layouts and the prepared marker/entry ABI layouts
/// to the final WIRE_LAYOUT. wire_core_layout also supports refusal diagnostics:
/// a guest presenting it without the prepared component is refused, never accepted.
fn layout(surface: &Surface<'_>) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut feed = |text: &str| {
        for byte in text.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    // Resource-control values are part of the interface even though they do not
    // appear in a catalog signature. Older guests may have allocated identity zero.
    feed(
        "resource generation zero reserved; Any identity zero is Unsupported/cross-group capability",
    );
    // The group's mods share one handler table, which lives in a word of the std member.
    feed(
        "group state: 32 words in the std member's horizon_group_state; 0 handler table, 1 cache budget, 2 eviction guard, 3.. eviction callbacks; members allocate through an evicting global allocator",
    );
    feed("static replies: bounded acknowledged ids or uncached text");
    feed("ConfigFiles imports: u8 root selector, 0 config, 1 data, 2 world; other values refused");
    feed("Hooks::subscribed: Vec<(hook name, i32 priority, bool receive_cancelled)>");
    for method in &surface.methods {
        feed(&format!("{method:?}"));
    }
    for def in &surface.types.defs {
        // Private borrowed descriptors also define bytes and checked writeback policy.
        // Hash the complete definition so private writeback contracts affect compatibility.
        feed(&format!("{def:?}"));
    }
    feed(&format!("{:?}", surface.hooks));
    hash
}

#[cfg(test)]
mod tests {
    /// The generated impls depend on the hand-written `Wire` impls the generator skips, so an
    /// edit to that file must rerun the build script.
    #[test]
    #[ignore = "daily"]
    fn enchantment_damage_registrar_and_callback_cross_to_wire() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
                let included = [
                    ("mod_api_generated.rs", generated.hooks.as_str()),
                    ("mod_api_generated_host.rs", generated.host.as_str()),
                    ("mod_api_host_areas.rs", generated.areas.as_str()),
                ];
                let providers = crate::capability_providers::generate(&root, &included)
                    .unwrap()
                    .0;
                let mut surface_included = included.to_vec();
                surface_included.push(("capability_providers_generated.rs", providers.as_str()));
                let surface = super::surface(&root, &surface_included).unwrap();
                for (owner, name) in [
                    ("EnchantmentDamageEffects", "register"),
                    ("EnchantmentDamageEffect", "apply"),
                    ("Hooks", "register_enchantment_damage_effects"),
                ] {
                    assert!(
                        surface
                            .methods
                            .iter()
                            .any(|method| method.owner == owner && method.name == name)
                    );
                }
                super::generate(&root, &included).unwrap();
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_worldgen_part_method_crosses_and_waits_for_its_group_as_a_worker_call() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
                let included = [
                    ("mod_api_generated.rs", generated.hooks.as_str()),
                    ("mod_api_generated_host.rs", generated.host.as_str()),
                    ("mod_api_host_areas.rs", generated.areas.as_str()),
                ];
                let providers = crate::capability_providers::generate(&root, &included)
                    .unwrap()
                    .0;
                let mut surface_included = included.to_vec();
                surface_included.push(("capability_providers_generated.rs", providers.as_str()));
                let surface = super::surface(&root, &surface_included).unwrap();
                let class = |name: &str| {
                    let method = surface
                        .methods
                        .iter()
                        .find(|method| method.owner == "BlockPart" && method.name == name)
                        .unwrap_or_else(|| panic!("BlockPart::{name} does not cross"));
                    super::call_class(&surface.types, method)
                };
                assert_eq!(class("on_tree_grow"), "super::Call::Worker");
                assert_eq!(class("is_ladder"), "super::Call::Hook");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn the_hand_written_wire_impls_are_tracked_sources() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
                let included = [
                    ("mod_api_generated.rs", generated.hooks.as_str()),
                    ("mod_api_generated_host.rs", generated.host.as_str()),
                    ("mod_api_host_areas.rs", generated.areas.as_str()),
                ];
                let sources = super::generate(&root, &included).unwrap().sources;
                assert!(
                    sources
                        .iter()
                        .any(|p| p.ends_with("crates/horizon-mod-vocab/src/wire/wire_types.rs"))
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    #[ignore = "daily"]
    fn layout_includes_private_borrowed_shapes() {
        let run = || {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
            let providers = crate::capability_providers::generate(&root, &[]).unwrap().0;
            let included = [
                ("mod_api_generated.rs", generated.hooks.as_str()),
                ("mod_api_generated_host.rs", generated.host.as_str()),
                ("mod_api_host_areas.rs", generated.areas.as_str()),
                ("capability_providers_generated.rs", providers.as_str()),
            ];
            let mut surface = super::surface(&root, &included).unwrap();
            let baseline = super::layout(&surface);
            assert!(
                surface
                    .types
                    .defs
                    .iter()
                    .filter(|def| def.lifetimes == 0)
                    .all(|def| matches!(def.borrowed, super::ty::BorrowedShape::None)),
                "owned fields must not be classified by the call generator"
            );
            let append = surface
                .types
                .defs
                .iter()
                .position(|d| d.append.is_some())
                .unwrap();
            assert!(matches!(
                surface.types.defs[append].borrowed,
                super::ty::BorrowedShape::None
            ));
            surface.types.defs[append].append.as_mut().unwrap().field += 1;
            assert_ne!(baseline, super::layout(&surface));
            surface.types.defs[append].append.as_mut().unwrap().field -= 1;
            let projection = surface
                .types
                .defs
                .iter_mut()
                .find(|d| d.projection.is_some())
                .unwrap();
            assert!(matches!(
                projection.borrowed,
                super::ty::BorrowedShape::None
            ));
            projection.projection.as_mut().unwrap()[0].ty = super::ty::Ty::Unit;
            assert_ne!(baseline, super::layout(&surface));
            let before_borrowed = super::layout(&surface);
            let borrowed = surface
                .types
                .defs
                .iter_mut()
                .find_map(|def| {
                    if def.lifetimes == 0 {
                        return None;
                    }
                    match &mut def.borrowed {
                        super::ty::BorrowedShape::Record(fields) => fields.first_mut(),
                        _ => None,
                    }
                })
                .expect("a borrowed call record");
            borrowed.ty = super::ty::Ty::Unit;
            assert_ne!(before_borrowed, super::layout(&surface));
        };
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(run)
            .unwrap()
            .join()
            .unwrap();
    }
    /// The book renders the coverage page from the generator at build time; what the page
    /// promises is checked here, against the generator itself.
    #[test]
    #[ignore = "daily"]
    fn coverage_page_is_well_formed() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let page = super::generate_coverage(&root).unwrap();
                let mut lines = page.lines();
                assert_eq!(lines.next(), Some("# Wire boundary coverage"));
                let header = "| Method | Status | Reason |";
                let rows: Vec<Vec<&str>> = page
                    .lines()
                    .skip_while(|line| *line != header)
                    .skip(2)
                    .take_while(|line| line.starts_with('|'))
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| line.trim_matches('|').split(" | ").map(str::trim).collect())
                    .collect();
                assert!(
                    rows.len() > 100,
                    "the table lists the surface: {} rows",
                    rows.len()
                );
                let mut methods = std::collections::BTreeSet::new();
                for row in &rows {
                    assert_eq!(row.len(), 3, "three cells: {row:?}");
                    assert!(
                        row.iter().all(|cell| !cell.is_empty()),
                        "no empty cell: {row:?}"
                    );
                    assert!(matches!(row[1], "crosses" | "gap"), "a status: {row:?}");
                    assert!(methods.insert(row[0]), "each method once: {}", row[0]);
                }
                for owner in ["Host", "ServerHost", "ClientHost", "Registrar"] {
                    assert!(
                        methods
                            .iter()
                            .any(|m| m.starts_with(&format!("`{owner}::"))),
                        "the page lists {owner}'s methods"
                    );
                }
                assert!(
                    page.contains("Required gap total: 0."),
                    "required boundary methods must not silently lose coverage"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
