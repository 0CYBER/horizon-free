//! The WebAssembly interface of the mod API: every method of the traits a mod calls (the hosts,
//! the backends, the drawing contexts) and every method it implements (`Hooks`, the parts, the
//! capability handlers), through one shared-memory boundary and the glue on both sides of it.
mod append;
mod borrowed;
mod calls;
mod host;
mod names;
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
    "Profiles",
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
    ("Profiles", "server_host_mut", "Ok(h.profiles())"),
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
    names::check_names(&surface.methods)?;
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
    let (codecs, codec_sources) = codecs(root)?;
    // The transport's own payloads, by the aliases both sides use: no call's signature names
    // them. `HOST_RELEASE` takes one `Id`; `HOST_LOG` and a refusal's reason are a `String`.
    let control = [
        "crate::wire::InitInput",
        "crate::wire::ReleaseInput",
        "crate::wire::Id",
        "String",
        "crate::wire::CallsAnswer",
    ]
    .iter()
    .map(|ty| format!("hash = crate::wire::layout_mix(hash, <{ty} as crate::wire::Wire>::layout(&mut vec![]));"))
    .collect::<String>();
    let _ = writeln!(
        wire,
        "/// What every mod of a group and the game must agree on exactly; each call is checked on\n/// its own (`call_layouts`).\n#[doc(hidden)]\npub fn wire_core_layout() -> u64 {{ wire_core_layout_with(crate::wire::PROTOCOL) }}\n/// [`wire_core_layout`] with `protocol` for the transport's values.\n#[doc(hidden)]\npub fn wire_core_layout_with(protocol: &[(&str, u64)]) -> u64 {{ let mut hash = {:#x}; {control} for &(name, value) in protocol {{ hash = crate::wire::layout_mix(crate::wire::layout_mix(hash, crate::wire::layout_name(name)), value); }} hash }}\nconst _: () = assert!(crate::wire::PREPARE == {:#x} && crate::wire::RELEASE_PREPARED == {:#x}, \"the generator's prepared ids are the transport's\");",
        core(codecs, FILE_ROOTS),
        prepared::PREPARE,
        prepared::RELEASE,
    );
    wire.push_str(&names::layouts(
        &surface.types,
        &surface.methods,
        &surface.dropped,
    ));
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
    let mut sources = surface.types.index.read.take();
    sources.extend(codec_sources);
    Ok(Generated {
        wire,
        guest,
        sources,
    })
}

/// The owned types the transport's control payloads reach, which no call names: `InitInput`, what
/// a mod's start receives, is a list of `PackageInfo`. The core hash mixes in their layouts.
const CONTROL_OWNED: &[&str] = &["mod_api::PackageInfo"];

/// The owned types the core reaches, by the path the glue names them by: the control payloads'
/// and those of the prepared hooks' entries (`reads:Vec<crate::EquipReadRow>`), sorted.
fn core_owned(abis: &[prepared::Abi]) -> Vec<String> {
    let mut paths: std::collections::BTreeSet<String> = CONTROL_OWNED
        .iter()
        .map(|path| (*path).to_owned())
        .collect();
    for entry in abis.iter().flat_map(|abi| &abi.entries) {
        for argument in entry.input.iter().chain(&entry.output) {
            let ty = argument.split_once(':').map_or(*argument, |(_, ty)| ty);
            let ty = ty
                .strip_prefix("Vec<")
                .and_then(|ty| ty.strip_suffix('>'))
                .unwrap_or(ty);
            // `crate::wire::Id` is a transport value; scalars and tuples have no owned type.
            if let Some(name) = ty.strip_prefix("crate::")
                && !name.starts_with("wire::")
            {
                paths.insert(format!("mod_api::{name}"));
            }
        }
    }
    paths.into_iter().collect()
}

/// What a wasm mod and the game check each other by when it loads (`mod_surface::boundary`):
/// every call's key, signature hash and the owned types it reaches, as `call_layouts` mixes them,
/// a fingerprint of the core hash's inputs that the source holds (the hand-written codecs and
/// the transport values `codecs` reads, and the prepared hooks' ABI) and the owned types the
/// core reaches: the control payloads' and the prepared hooks' entries' (`prepared_abi_layout`
/// mixes in each entry type's layout). The owned types' shapes are left for the surface export
/// to fill in.
pub(crate) fn boundary(
    root: &Path,
    surface: &Surface<'_>,
) -> Result<mod_surface::boundary::Boundary, String> {
    let prepared = crate::Catalog::load(root)?
        .hooks
        .into_iter()
        .filter_map(|hook| hook.prepared.map(|field| (hook.name, field)))
        .collect::<Vec<_>>();
    let abis = prepared::describe(&surface.methods, &prepared)?;
    let (codecs, _) = codecs(root)?;
    let core = names::fnv(&format!(
        "{:#x};{:#x};{:#x};{}",
        core(codecs, FILE_ROOTS),
        prepared::PREPARE,
        prepared::RELEASE,
        prepared::layout_code(&abis)
    ));
    let calls = names::calls(&surface.types, &surface.methods, &surface.dropped)
        .into_iter()
        .map(
            |(key, signature, found, export)| mod_surface::boundary::Call {
                key,
                signature: format!("{signature:016x}"),
                export,
                owned: found
                    .iter()
                    .map(|def| mod_surface::boundary::Owned {
                        path: surface.types.defs[*def].rust.clone(),
                        shape: None,
                    })
                    .collect(),
            },
        )
        .collect();
    Ok(mod_surface::boundary::Boundary {
        schema: mod_surface::boundary::SCHEMA,
        core: format!("{core:016x}"),
        core_owned: core_owned(&abis)
            .into_iter()
            .map(|path| mod_surface::boundary::Owned { path, shape: None })
            .collect(),
        calls,
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

/// The seed of the core hash: the transport's rules no call's signature carries, and the
/// hand-written codecs (`codecs`), whose bytes no declared layout describes. The calls
/// themselves are checked one by one (`names`), so an added call leaves it as it was.
fn core(codecs: u64, file_roots: &[(u8, &str, &str)]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut feed = |text: &str| {
        for byte in text.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    // A new value here refuses every mod built before it, those of the positional ABI first.
    feed(
        "calls by name (#18253): imports horizon.<owner>::<name>#<signature>, horizon.host for the reserved ids only; exports by name-hash id; CALLS answers call_layouts",
    );
    // Resource-control values are part of the interface even though they do not
    // appear in a catalog signature. Older guests may have allocated identity zero.
    feed(
        "resource generation zero reserved; Any identity zero is Unsupported/cross-group capability",
    );
    // The reserved ids and the group state's words are mixed in at run time from the
    // constants the transport uses (`crate::wire::PROTOCOL`), not described here.
    // Behaviour, not a value: what a member's allocator does with the group's budget.
    feed("members allocate through an evicting global allocator");
    // The selector table the generated guest proxy and host dispatch are both written from.
    feed(&format!("file roots {file_roots:?}"));
    feed(&format!("hand-written codecs {codecs:#x}"));
    hash
}

/// A hash of every hand-written `Wire` codec of the API (`impl Wire for …` and the macros that
/// write one), doc comments aside, and the files read for it. Their bytes are what the code
/// says, which no declared layout describes, so a change to one refuses every mod built before.
fn codecs(root: &Path) -> Result<(u64, Vec<std::path::PathBuf>), String> {
    fn strip_docs(tokens: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
        use proc_macro2::TokenTree;
        let mut out = Vec::new();
        let mut trees = tokens.into_iter().peekable();
        while let Some(tree) = trees.next() {
            if let TokenTree::Punct(punct) = &tree
                && punct.as_char() == '#'
                && let Some(TokenTree::Group(group)) = trees.peek()
                && group.delimiter() == proc_macro2::Delimiter::Bracket
                && group
                    .stream()
                    .into_iter()
                    .next()
                    .is_some_and(|first| first.to_string() == "doc")
            {
                trees.next();
                continue;
            }
            out.push(match tree {
                TokenTree::Group(group) => {
                    let mut inner =
                        proc_macro2::Group::new(group.delimiter(), strip_docs(group.stream()));
                    inner.set_span(group.span());
                    TokenTree::Group(inner)
                }
                tree => tree,
            });
        }
        out.into_iter().collect()
    }
    fn test_only(attrs: &[syn::Attribute]) -> bool {
        attrs.iter().any(|attr| {
            attr.path().is_ident("cfg")
                && quote::ToTokens::to_token_stream(attr)
                    .to_string()
                    .contains("test")
        })
    }
    fn attrs(item: &Item) -> &[syn::Attribute] {
        match item {
            Item::Const(item) => &item.attrs,
            Item::Enum(item) => &item.attrs,
            Item::Fn(item) => &item.attrs,
            Item::Impl(item) => &item.attrs,
            Item::Macro(item) => &item.attrs,
            Item::Mod(item) => &item.attrs,
            Item::Static(item) => &item.attrs,
            Item::Struct(item) => &item.attrs,
            Item::Trait(item) => &item.attrs,
            Item::Type(item) => &item.attrs,
            Item::Use(item) => &item.attrs,
            _ => &[],
        }
    }
    /// Every item of a codec module (`horizon-mod-vocab/src/wire`): the helpers its `Wire`
    /// impls delegate to (`tag_tree`, `json_tree`) encode as much as the impls do.
    fn whole(list: &[Item], texts: &mut Vec<String>) {
        for item in list.iter().filter(|item| !test_only(attrs(item))) {
            match item {
                Item::Mod(module) => {
                    if let Some((_, content)) = &module.content {
                        whole(content, texts);
                    }
                }
                item => texts.push(strip_docs(quote::ToTokens::to_token_stream(item)).to_string()),
            }
        }
    }
    /// The `serde` shape of every item in a file holding a `#[wire(via = String)]` type: its
    /// wire bytes are JSON written by `serde` (`MeshDefinition`), which no layout describes.
    fn json(list: &[Item], texts: &mut Vec<String>) {
        for item in list.iter().filter(|item| !test_only(attrs(item))) {
            let serde = attrs(item).iter().any(|attr| {
                attr.path().is_ident("derive")
                    && ["Serialize", "Deserialize"].iter().any(|derive| {
                        quote::ToTokens::to_token_stream(attr)
                            .to_string()
                            .contains(derive)
                    })
                    || attr.path().is_ident("serde")
            });
            if serde {
                texts.push(strip_docs(quote::ToTokens::to_token_stream(item)).to_string());
            }
        }
    }
    fn items(list: &[Item], texts: &mut Vec<String>) {
        for item in list {
            match item {
                Item::Impl(block)
                    if !test_only(&block.attrs)
                        && block.trait_.as_ref().is_some_and(|(_, path, _)| {
                            path.segments.last().is_some_and(|s| s.ident == "Wire")
                        }) =>
                {
                    texts.push(strip_docs(quote::ToTokens::to_token_stream(block)).to_string());
                }
                Item::Macro(mac)
                    if mac.mac.path.is_ident("macro_rules")
                        && mac.mac.tokens.to_string().contains("Wire for") =>
                {
                    texts.push(strip_docs(mac.mac.tokens.clone()).to_string());
                }
                Item::Mod(module) if !test_only(&module.attrs) => {
                    if let Some((_, content)) = &module.content {
                        items(content, texts);
                    }
                }
                _ => {}
            }
        }
    }
    fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                walk(&path, files)?;
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
        Ok(())
    }
    let dirs = [
        root.join("crates/horizon-mod-vocab/src"),
        root.join("crates/horizon-mod-api/src"),
    ];
    let mut files = Vec::new();
    for dir in &dirs {
        walk(dir, &mut files)?;
    }
    let mut keyed: Vec<(String, std::path::PathBuf)> = files
        .into_iter()
        .map(|path| {
            let key = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            (key, path)
        })
        .filter(|(key, _)| !key.contains("tests") && !key.contains("golden"))
        .collect();
    keyed.sort();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for (key, path) in &keyed {
        // Line ends as Git stores them: a literal spanning lines must hash the same in a
        // Windows checkout as in the SDK's.
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("{key}: {e}"))?
            .replace("\r\n", "\n");
        let codec_module = key.starts_with("crates/horizon-mod-vocab/src/wire/");
        let via_json = text.contains("wire(via = String)");
        if !codec_module && !via_json && !text.contains("Wire for") {
            continue;
        }
        let file = syn::parse_file(&text).map_err(|e| format!("{key}: {e}"))?;
        let mut texts = Vec::new();
        if codec_module {
            whole(&file.items, &mut texts);
        } else {
            items(&file.items, &mut texts);
        }
        if via_json {
            json(&file.items, &mut texts);
        }
        for text in &texts {
            for byte in text.bytes().chain([0]) {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    // The network NBT that `AttachmentBatch` embeds as bytes: its tag ids and the limits both
    // sides read and write it with. The rest of the codec is vanilla's format.
    let nbt = root.join(NBT);
    let text = std::fs::read_to_string(&nbt)
        .map_err(|e| format!("{NBT}: {e}"))?
        .replace("\r\n", "\n");
    let file = syn::parse_file(&text).map_err(|e| format!("{NBT}: {e}"))?;
    let mut found = 0;
    for item in &file.items {
        let Item::Impl(block) = item else { continue };
        let target = quote::ToTokens::to_token_stream(&block.self_ty).to_string();
        let texts: Vec<String> = match &block.trait_ {
            Some((_, path, _)) if target == "Limits" && path.is_ident("Default") => {
                vec![quote::ToTokens::to_token_stream(block).to_string()]
            }
            None if target == "Tag" => block
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::ImplItem::Fn(method) if method.sig.ident == "id" => {
                        Some(strip_docs(quote::ToTokens::to_token_stream(method)).to_string())
                    }
                    _ => None,
                })
                .collect(),
            _ => continue,
        };
        for text in texts {
            found += 1;
            for byte in text.bytes().chain([0]) {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    if found != 2 {
        return Err(format!("{NBT}: no `Tag::id` or `Limits::default` to hash"));
    }
    // The directories, so a codec in a new file reruns the build script too.
    let mut sources = dirs.to_vec();
    sources.push(nbt);
    Ok((hash, sources))
}

/// Where the network NBT codec lives (`codecs`).
const NBT: &str = "crates/horizon-core/src/nbt.rs";

#[cfg(test)]
mod tests {
    /// The core hash covers what a codec module encodes beyond its `impl Wire` blocks
    /// (`tag_tree`'s NBT discriminants), the same in a CRLF checkout as in an LF one.
    #[test]
    fn the_core_hash_covers_codec_helpers_whatever_the_line_ends() {
        let root = std::env::temp_dir().join(format!("horizon-codecs-{}", std::process::id()));
        let wire = root.join("crates/horizon-mod-vocab/src/wire");
        std::fs::create_dir_all(&wire).unwrap();
        std::fs::create_dir_all(root.join("crates/horizon-mod-api/src")).unwrap();
        std::fs::create_dir_all(root.join("crates/horizon-core/src")).unwrap();
        std::fs::write(root.join(super::NBT), NBT).unwrap();
        let hash = |tag_tree: &str| {
            std::fs::write(wire.join("tag_tree.rs"), tag_tree).unwrap();
            super::codecs(&root).unwrap().0
        };
        let helper = "pub fn tag(kind: u8) -> u8 {\n    match kind { 1 => 10, _ => 0 }\n}\nconst NOTE: &str = \"one\ntwo\";\n";
        let before = hash(helper);
        assert_ne!(before, hash(&helper.replace("10", "11")));
        assert_eq!(before, hash(&helper.replace('\n', "\r\n")));
        // The embedded network NBT's tag ids and limits.
        let nbt = |text: &str| {
            std::fs::write(root.join(super::NBT), text).unwrap();
            hash(helper)
        };
        assert_eq!(before, nbt(&NBT.replace("fn get", "fn find")));
        assert_ne!(before, nbt(&NBT.replace("=> 3", "=> 13")));
        assert_ne!(before, nbt(&NBT.replace("512", "511")));
        std::fs::write(root.join(super::NBT), NBT).unwrap();
        // The JSON shape of a type crossing as a string: its serde items, not the file's code.
        let models = root.join("crates/horizon-mod-vocab/src/models.rs");
        let json = |text: &str| {
            std::fs::write(&models, text).unwrap();
            hash(helper)
        };
        let shape = "#[derive(serde::Serialize, mod_api::Wire)]\n#[wire(via = String)]\npub struct Mesh { parts: Vec<Part> }\n#[derive(serde::Deserialize)]\nstruct Part { size: f32 }\nfn helper() {}\n";
        let with_shape = json(shape);
        assert_eq!(with_shape, json(&shape.replace("fn helper", "fn other")));
        assert_ne!(with_shape, json(&shape.replace("size", "width")));
        std::fs::remove_dir_all(&root).ok();
    }

    const NBT: &str = "impl Tag {\n    pub fn id(&self) -> u8 { match self { Self::End => 0, Self::Int(_) => 3 } }\n    pub fn get(&self) {}\n}\nimpl Default for Limits {\n    fn default() -> Self { Self { max_depth: 512 } }\n}\n";

    /// `codecs` reads a file outside the codec module only when its text says `Wire for`, so an
    /// `impl Wire` whose header splits that over two lines would cross unhashed (#18400). Every
    /// hand-written codec of the tree it reads must be in a file it reads.
    #[test]
    fn every_hand_written_codec_is_in_a_file_the_core_hash_reads() {
        use syn::Item;
        fn test_only(attrs: &[syn::Attribute]) -> bool {
            attrs.iter().any(|attr| {
                attr.path().is_ident("cfg")
                    && quote::ToTokens::to_token_stream(attr)
                        .to_string()
                        .contains("test")
            })
        }
        /// Whether `items` hold what `codecs`' `items` hashes: a `Wire` impl or a macro writing one.
        fn codec(items: &[Item]) -> bool {
            items.iter().any(|item| match item {
                Item::Impl(block) => {
                    !test_only(&block.attrs)
                        && block.trait_.as_ref().is_some_and(|(_, path, _)| {
                            path.segments.last().is_some_and(|s| s.ident == "Wire")
                        })
                }
                Item::Macro(mac) => {
                    mac.mac.path.is_ident("macro_rules")
                        && mac.mac.tokens.to_string().contains("Wire for")
                }
                Item::Mod(module) if !test_only(&module.attrs) => module
                    .content
                    .as_ref()
                    .is_some_and(|(_, content)| codec(content)),
                _ => false,
            })
        }
        fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, files);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    files.push(path);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        // `codecs`' directories; below, the files it skips and the codec module it reads whole.
        for dir in ["crates/horizon-mod-vocab/src", "crates/horizon-mod-api/src"] {
            walk(&root.join(dir), &mut files);
        }
        let (mut read, mut unread) = (0, Vec::new());
        for path in files {
            let key = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if key.contains("tests")
                || key.contains("golden")
                || key.starts_with("crates/horizon-mod-vocab/src/wire/")
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            if !codec(&syn::parse_file(&text).unwrap().items) {
                continue;
            }
            // Read by the production filter: the file's codecs move the hash `codecs` gives.
            if core_hash_of(&key, &text) != core_hash_of(&key, "") {
                read += 1;
            } else {
                unread.push(key);
            }
        }
        assert!(
            unread.is_empty(),
            "codecs the core hash doesn't read: {unread:?}"
        );
        assert!(
            read > 0,
            "no hand-written codec found outside the codec module"
        );
    }

    /// `codecs`' hash of a tree holding only `text` at `key` (and the real network NBT codec).
    fn core_hash_of(key: &str, text: &str) -> u64 {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static RUN: AtomicUsize = AtomicUsize::new(0);
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let root = std::env::temp_dir().join(format!(
            "horizon-core-hash-{}-{}",
            std::process::id(),
            RUN.fetch_add(1, Ordering::Relaxed)
        ));
        let file = root.join(key);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join("crates/horizon-mod-vocab/src")).unwrap();
        std::fs::create_dir_all(root.join("crates/horizon-mod-api/src")).unwrap();
        std::fs::create_dir_all(root.join(super::NBT).parent().unwrap()).unwrap();
        std::fs::copy(repo.join(super::NBT), root.join(super::NBT)).unwrap();
        std::fs::write(&file, text).unwrap();
        let hash = super::codecs(&root).unwrap().0;
        std::fs::remove_dir_all(&root).ok();
        hash
    }

    /// A codec written with a qualified path (`impl crate::wire::Wire for …`) is in the core
    /// hash: an edit to its body alone, the bytes it writes, moves it (#18400).
    #[test]
    fn a_qualified_wire_impls_body_is_in_the_core_hash() {
        let key = "crates/horizon-mod-vocab/src/entity/attachment_query.rs";
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let text = std::fs::read_to_string(repo.join(key)).unwrap();
        assert!(text.contains("impl crate::wire::Wire for AttachmentWriteToken {"));
        let body = "out.put(&generation);";
        assert_eq!(text.matches(body).count(), 1);
        assert_ne!(
            core_hash_of(key, &text),
            core_hash_of(key, &text.replace(body, "out.put(&(generation + 1));"))
        );
    }

    /// A wasm guest builds an `EntityTypeKey` from its slot without the native `from_slot`
    /// (`entity/types.rs`), in code neither hash reads and no native test runs (#18400). It is
    /// pinned here as written; it is the only wasm-only branch of the projections
    /// `wire_layout_tests` covers outside the codec module (`AttachmentBatch::query`'s is no codec).
    #[test]
    fn a_guest_keeps_an_entity_type_keys_slot() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let wasm_only = "#[cfg(target_arch = \"wasm32\")]";
        let read = |key: &str| std::fs::read_to_string(repo.join(key)).unwrap();
        let branches: Vec<(&str, usize)> = [
            "crates/horizon-core/src/identifier.rs",
            "crates/horizon-mod-runtime/src/attachments/index.rs",
            "crates/horizon-mod-vocab/src/client/keyframe_animation.rs",
            "crates/horizon-mod-vocab/src/client/model_instance.rs",
            "crates/horizon-mod-vocab/src/client/posed_parts.rs",
            "crates/horizon-mod-vocab/src/client/render_pipeline.rs",
            "crates/horizon-mod-vocab/src/content/registry_keys.rs",
            "crates/horizon-mod-vocab/src/entity/attachment_query.rs",
            "crates/horizon-mod-vocab/src/entity/types.rs",
            "crates/horizon-mod-vocab/src/item/item_stack.rs",
            "crates/horizon-mod-vocab/src/item/storage.rs",
            "crates/horizon-mod-vocab/src/item/types.rs",
            "crates/horizon-mod-vocab/src/world/ticket_controller.rs",
        ]
        .into_iter()
        .map(|key| (key, read(key).matches(wasm_only).count()))
        .filter(|(_, count)| *count > 0)
        .collect();
        assert_eq!(
            branches,
            [
                ("crates/horizon-mod-vocab/src/entity/attachment_query.rs", 1),
                ("crates/horizon-mod-vocab/src/entity/types.rs", 1),
            ],
            "a new wasm-only branch in a projection's helpers needs pinning here"
        );
        let file = syn::parse_file(&read("crates/horizon-mod-vocab/src/entity/types.rs")).unwrap();
        let try_from = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Impl(block)
                    if quote::ToTokens::to_token_stream(&block.self_ty).to_string()
                        == "EntityTypeKey"
                        && block.trait_.as_ref().is_some_and(|(_, path, _)| {
                            quote::ToTokens::to_token_stream(path).to_string() == "TryFrom < u32 >"
                        }) =>
                {
                    block.items.iter().find_map(|item| match item {
                        syn::ImplItem::Fn(f) if f.sig.ident == "try_from" => Some(f),
                        _ => None,
                    })
                }
                _ => None,
            })
            .expect("EntityTypeKey's TryFrom<u32>");
        let guest: Vec<String> = try_from
            .block
            .stmts
            .iter()
            .filter_map(|stmt| match stmt {
                syn::Stmt::Expr(syn::Expr::Block(block), _)
                    if block.attrs.iter().any(|attr| {
                        quote::ToTokens::to_token_stream(attr).to_string()
                            == "# [cfg (target_arch = \"wasm32\")]"
                    }) =>
                {
                    Some(quote::ToTokens::to_token_stream(&block.block).to_string())
                }
                _ => None,
            })
            .collect();
        let expected = syn::parse_str::<syn::Block>("{ Ok(Self(slot)) }").unwrap();
        assert_eq!(
            guest,
            [quote::ToTokens::to_token_stream(&expected).to_string()]
        );
    }

    /// The core hash covers the file roots' selectors: swapping two refuses mods built before.
    #[test]
    fn swapped_file_roots_change_the_core_hash() {
        let mut roots = super::FILE_ROOTS.to_vec();
        let before = super::core(0, &roots);
        let (config, data) = (roots[0].0, roots[1].0);
        roots[0].0 = data;
        roots[1].0 = config;
        assert_ne!(before, super::core(0, &roots));
    }

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
        // What the calls carry: each call's signature, which covers the borrowed shapes.
        fn layout(surface: &super::Surface<'_>) -> Vec<u64> {
            surface
                .methods
                .iter()
                .map(|m| super::names::signature(&surface.types, m))
                .collect()
        }
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
            let baseline = layout(&surface);
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
            assert_ne!(baseline, layout(&surface));
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
            assert_ne!(baseline, layout(&surface));
            let before_borrowed = layout(&surface);
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
            assert_ne!(before_borrowed, layout(&surface));
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
