//! Generate the typed mod interface from native hook signatures and catalog contracts.
mod catalog;
mod dispatch_fixtures;
mod exports;
mod game_modules;
mod interface_cache;
#[cfg(test)]
mod interface_cache_tests;
#[cfg(test)]
mod listener_output_fixture;
#[cfg(feature = "mixed-output-fixture")]
mod mixed_output_fixture;
#[doc(hidden)]
pub use dispatch_fixtures::layout as dispatch_fixture_layout;
pub use game_modules::materialize_game_modules;
mod hook_targets;
mod host;
mod layout;
mod named_hooks;
mod parts;
pub use catalog::DECLARATIONS;
use catalog::{Catalog, Contract, DrawContext, Hook, HostAccess, PartKind, PhaseContext, Side};
use heck::ToSnakeCase;
pub use layout::native_hook_layout;
use quote::ToTokens;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use syn::{FnArg, ImplItem, Item, Pat, Type, visit::Visit, visit_mut::VisitMut};

fn impl_functions(item: syn::ItemImpl) -> impl Iterator<Item = syn::ItemFn> {
    item.items.into_iter().filter_map(|item| match item {
        ImplItem::Fn(method) => Some(syn::ItemFn {
            attrs: method.attrs,
            vis: method.vis,
            sig: method.sig,
            block: Box::new(method.block),
        }),
        _ => None,
    })
}

fn doc_comments(
    attrs: &[syn::Attribute],
    name: &str,
    preserve_empty: bool,
) -> Result<String, String> {
    let mut docs = String::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("doc")) {
        let syn::Meta::NameValue(value) = &attr.meta else {
            return Err(format!("unsupported doc attribute in {name}"));
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(doc),
            ..
        }) = &value.value
        else {
            return Err(format!("unsupported doc attribute in {name}"));
        };
        let value = doc.value();
        for line in value.split('\n') {
            if preserve_empty || !line.is_empty() {
                docs.push_str(&format!("///{line}\n"));
            }
        }
    }
    Ok(docs)
}

/// The mod API's public `*Ref`, `*Key` and `*Ticket` handles, by their receiver name.
struct ApiTypes {
    handles: BTreeMap<String, String>,
}

fn api_types(source: &str) -> Result<ApiTypes, String> {
    let file = syn::parse_file(source).map_err(|e| e.to_string())?;
    let mut handles = BTreeMap::new();
    for item in file.items {
        let Item::Struct(item) = item else {
            continue;
        };
        if !matches!(item.vis, syn::Visibility::Public(_)) {
            continue;
        }
        let name = item.ident.to_string();
        let base = name
            .strip_suffix("Ref")
            .map(str::to_owned)
            .or_else(|| (name.ends_with("Key") || name.ends_with("Ticket")).then(|| name.clone()));
        if let Some(base) = base.filter(|base| !base.is_empty()) {
            let receiver = base.to_snake_case();
            if handles.insert(receiver.clone(), name).is_some() {
                return Err(format!("duplicate handle receiver {receiver}"));
            }
        }
    }
    Ok(ApiTypes { handles })
}

fn has_call(block: &syn::Block, expected: &str) -> bool {
    struct Calls<'a> {
        expected: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Calls<'_> {
        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            if call.method == self.expected {
                self.found = true;
            }
            syn::visit::visit_expr_method_call(self, call);
        }
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(path) = &*call.func
                && path
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == self.expected)
            {
                self.found = true;
            }
            syn::visit::visit_expr_call(self, call);
        }
    }
    let mut calls = Calls {
        expected,
        found: false,
    };
    calls.visit_block(block);
    calls.found
}

fn one_function<'a>(
    items: impl Iterator<Item = &'a syn::ItemFn>,
    name: &str,
) -> Result<&'a syn::ItemFn, String> {
    let mut matches = items.filter(|item| item.sig.ident == name);
    let first = matches
        .next()
        .ok_or_else(|| format!("expected one {name} signature"))?;
    if matches.next().is_some() {
        return Err(format!("expected one {name} signature"));
    }
    Ok(first)
}

fn fresh_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut name = base.to_owned();
    while !used.insert(name.clone()) {
        name.push('_');
    }
    name
}

/// Turns a hook argument's type into its event field's. A `&mut` primitive scalar becomes the value
/// itself, written back after every mod has run; any other `&mut` stays a reference to the caller's
/// value. Either way one event goes through every mod, so each sees the changes before it. Every
/// elided reference lifetime becomes the event's `'a`, or `'b` inside a `&mut`, which can't shorten
/// it.
#[derive(Default)]
struct EventLifetimes {
    a: bool,
    b: bool,
    in_mut: bool,
}

/// The primitive types a `&mut` argument crosses as a copied value — every one of them `Copy`.
pub(crate) const PRIMITIVE_SCALARS: &[&str] = &[
    "bool", "char", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128",
    "usize", "f32", "f64",
];

/// Whether `ty` names one of [`PRIMITIVE_SCALARS`].
pub(crate) fn is_primitive_scalar(ty: &Type) -> bool {
    matches!(ty, Type::Path(path)
        if path.qself.is_none()
            && path
                .path
                .get_ident()
                .is_some_and(|ident| PRIMITIVE_SCALARS.contains(&ident.to_string().as_str())))
}

impl EventLifetimes {
    /// The field's type, and whether it is a scalar copied in and written back.
    fn field(&mut self, ty: &Type) -> (Type, bool) {
        let scalar = match ty {
            Type::Reference(reference)
                if reference.mutability.is_some() && is_primitive_scalar(&reference.elem) =>
            {
                Some(&*reference.elem)
            }
            _ => None,
        };
        if let Some(scalar) = scalar {
            return (scalar.clone(), true);
        }
        let mut field = ty.clone();
        self.visit_type_mut(&mut field);
        (field, false)
    }
}

impl VisitMut for EventLifetimes {
    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        if reference.lifetime.is_none() {
            reference.lifetime = Some(syn::parse_quote!('_));
        }
        let outer = self.in_mut;
        self.visit_lifetime_mut(reference.lifetime.as_mut().unwrap());
        self.in_mut |= reference.mutability.is_some();
        self.visit_type_mut(&mut reference.elem);
        self.in_mut = outer;
    }
    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
        if lifetime.ident == "_" {
            *lifetime = if self.in_mut {
                self.b = true;
                syn::parse_quote!('b)
            } else {
                self.a = true;
                syn::parse_quote!('a)
            };
        }
    }
}

impl DrawContext {
    /// The context whose surface `ty` borrows, and whether it borrows it mutably.
    fn of<'a>(contexts: &'a [DrawContext], ty: &Type) -> Option<(&'a DrawContext, bool)> {
        let Type::Reference(reference) = ty else {
            return None;
        };
        let Type::Path(path) = &*reference.elem else {
            return None;
        };
        let ident = path.path.segments.last()?.ident.to_string();
        contexts
            .iter()
            .find(|context| context.surface.rsplit("::").next() == Some(ident.as_str()))
            .map(|context| (context, reference.mutability.is_some()))
    }
    /// The game struct implementing the trait.
    fn game_struct(&self) -> String {
        format!("Game{}", self.trait_name)
    }
}

fn hook_id(name: &str) -> String {
    name.split('_')
        .map(|word| word[..1].to_uppercase() + &word[1..])
        .collect()
}

fn complete_hook_constants(
    code: String,
    trait_name: &str,
    module: &str,
    id: &str,
) -> Result<String, String> {
    let file = syn::parse_file(&code).map_err(|e| e.to_string())?;
    let methods = file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Trait(item) if item.ident == trait_name => Some(&item.items),
            _ => None,
        })
        .ok_or("generated hook trait missing")?;
    let existing: HashSet<_> = file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Mod(item) if item.ident == module => {
                item.content.as_ref().map(|(_, items)| items)
            }
            _ => None,
        })
        .ok_or("generated hook constants missing")?
        .iter()
        .filter_map(|item| match item {
            Item::Const(item) => Some(item.ident.to_string()),
            _ => None,
        })
        .collect();
    let mut defaults = String::new();
    for method in methods {
        if let syn::TraitItem::Fn(method) = method {
            let name = method.sig.ident.to_string();
            if !existing.contains(&name) {
                defaults.push_str(&format!("pub const {name}: Option<super::{id}> = None;\n"));
            }
        }
    }
    Ok(code.replacen(
        &format!("pub mod {module} {{"),
        &format!("pub mod {module} {{\n{defaults}"),
        1,
    ))
}

/// A catalog hook point, as a raw injection's `target` can name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookPoint {
    pub name: String,
    /// The contract version an injection's `hook_version` names.
    pub version: u64,
    /// The function the hook is called from: an injection's `target` for it.
    pub target: String,
    /// A scoped hook is a Part method alone and has no `Hooks` method.
    pub scoped: bool,
}

/// The catalog's hook points, from the declaration files, as the generator reads them.
pub fn hook_points(root: &Path) -> Result<Vec<HookPoint>, String> {
    Ok(Catalog::load(root)?
        .hooks
        .into_iter()
        .map(|hook| HookPoint {
            scoped: hook.scope.is_some(),
            name: hook.name,
            version: hook.version,
            target: hook.target,
        })
        .collect())
}

/// Serialize hook names and NeoForge counterparts for the port-pack event report.
/// The declaration loader remains the single source of these labels.
pub fn neoforge_catalog(root: &Path) -> Result<String, String> {
    let hooks: Vec<_> = Catalog::load(root)?
        .hooks
        .into_iter()
        .map(|hook| {
            serde_json::json!({
                "name": hook.name,
                "neoforge": hook.counterparts.neoforge.join(" / "),
            })
        })
        .collect();
    serde_json::to_string(&serde_json::json!({ "hooks": hooks })).map_err(|e| e.to_string())
}

/// The crate folder and module path of a hook target that is a free function
/// outside `Game`. `horizon-client-lib` builds from `horizon-client/src`
/// (`client_lib.rs`).
fn free_function_source(target: &str) -> Option<(&'static str, String)> {
    let prefixes = [
        ("horizon_world::", "horizon-world"),
        ("mod_api::", "horizon-mod-vocab"),
        ("mod_render::", "horizon-mod-render"),
        ("horizon_client_lib::", "horizon-client"),
        ("horizon_client_render::", "horizon-client-render"),
    ];
    prefixes
        .into_iter()
        .find_map(|(prefix, krate)| target.strip_prefix(prefix).map(|p| (krate, p.to_owned())))
}

/// Whether a hook function needs its dispatch `call` filled in: false when it
/// makes the call; true when it is a `Game` or `GameLevel` method whose body is
/// only the placeholder a new hook starts with, `let _ = (every, parameter);`
/// (`let _ = parameter;` for one). Any other body is an error, left alone.
fn needs_fill(method: &syn::ItemFn, call: &str, in_game: bool) -> Result<bool, String> {
    if has_call(&method.block, call)
        || call
            .strip_prefix("dispatch_")
            .is_some_and(|name| has_call(&method.block, &format!("fire_{name}")))
    {
        return Ok(false);
    }
    let params: Vec<String> = method
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(arg) => Some(arg.pat.to_token_stream().to_string()),
            FnArg::Receiver(_) => None,
        })
        .collect();
    let named = match &method.block.stmts[..] {
        [
            syn::Stmt::Local(syn::Local {
                pat: Pat::Wild(_),
                init:
                    Some(syn::LocalInit {
                        expr,
                        diverge: None,
                        ..
                    }),
                ..
            }),
        ] => match &**expr {
            syn::Expr::Tuple(tuple) => Some(
                tuple
                    .elems
                    .iter()
                    .map(|e| e.to_token_stream().to_string())
                    .collect(),
            ),
            single @ syn::Expr::Path(_) => Some(vec![single.to_token_stream().to_string()]),
            _ => None,
        },
        _ => None,
    };
    if in_game && named == Some(params) {
        return Ok(true);
    }
    Err(format!(
        "missing {call} call in {}: add it, or leave a Game hook's body as the let _ = (…); placeholder for cargo run -p xtask -- mod-api to fill",
        method.sig.ident
    ))
}

/// A hook function whose placeholder body becomes `call`.
#[derive(Clone)]
struct Fill {
    native: String,
    call: String,
}

/// `source` with each fill's placeholder statement replaced by its call,
/// formatted as `cargo fmt` would.
fn filled_hooks(source: &str, fills: &[Fill]) -> Result<Vec<u8>, String> {
    let mut source = source.to_owned();
    for Fill { native, call } in fills {
        let start = source
            .find(&format!("fn {native}("))
            .ok_or_else(|| format!("{native} not found"))?;
        let placeholder = start
            + source[start..]
                .find("let _ =")
                .ok_or_else(|| format!("{native} has no placeholder"))?;
        let end = placeholder
            + source[placeholder..]
                .find(';')
                .ok_or_else(|| format!("{native} has no placeholder"))?
            + 1;
        source.replace_range(placeholder..end, call);
    }
    rustfmt(&source)
}

/// The workspace's edition, which rustfmt formats generated code for.
const EDITION: &str = "2024";

/// `code` as `cargo fmt` formats it.
fn rustfmt(code: &str) -> Result<Vec<u8>, String> {
    let mut child = Command::new("rustfmt")
        .args(["--emit", "stdout", "--edition", EDITION])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    child
        .stdin
        .take()
        .ok_or("rustfmt stdin")?
        .write_all(code.as_bytes())
        .map_err(|e| e.to_string())?;
    let formatted = child.wait_with_output().map_err(|e| e.to_string())?;
    if !formatted.status.success() {
        return Err("rustfmt failed".into());
    }
    Ok(formatted.stdout)
}

/// The doc of [`BlockPart`], the first part trait in the generated file.
const PART_DOCS: &str = "/// A block's part: code attached through [`Registrar`] to one block, a mod's own or vanilla's.\n/// Its methods are the scoped hooks', asked only for the blocks it is attached to: a scoped\n/// hook has no [`Hooks`] method. They may run on the window's thread or the server thread.\n/// Annotate each part trait implementation with [`horizon_part`] to derive its subscriptions.\n";

/// The doc of [`EntityPart`], the goal- and behavior-registering part trait (#1077, #824),
/// which the general entity tick moment (#792) also scopes hooks to.
const ENTITY_PART_DOCS: &str = "/// An entity type's part (arismoko/horizon#1077): code attached through\n/// `RegistrationContext::entity_part` to one entity type, a mod's own or vanilla's\n/// (`minecraft:zombie`). Its `register_goals` runs at every `Mob` construction of that\n/// type, spawned or loaded, right after the species' own goals — vanilla's `registerGoals`\n/// moment — and the goals it adds join the mob's two selectors beside vanilla's, same\n/// priorities and interruption. Its `register_behaviors` runs wherever a brain-driven\n/// species' brain is built (#824). Its `spawn_cluster_size` answers how large a spawn group of the\n/// type may grow. Its `general_entity_tick` runs for one general entity row's authoritative\n/// tick (#792). Annotate each part trait implementation with\n/// [`horizon_part`] to derive its subscriptions; the goal, behavior and spawn-size methods\n/// are not hooks, so they subscribe nothing whatever they override.\n";

/// The crate whose generated sources are being built.
#[derive(Clone, Copy)]
pub enum Target {
    /// `horizon-mod-api`: the mod-side WebAssembly guest transport.
    Api,
    /// Engine vocabulary: hook and host traits, values, and dispatch.
    Vocab,
    /// The game: the host traits' implementation on `Game`.
    Client,
}

/// Fill new hooks' placeholders in `game/native_hooks/`. Never writes generated files.
/// Named hooks in other logical game modules must already call their own dispatch;
/// this command validates them but never fills or rewrites their bodies.
pub fn fill_hooks(root: &Path) -> Result<(), String> {
    sources(root, FillMode::Fill).map(|_| ())
}

#[derive(Debug)]
struct Sources {
    hooks: String,
    host: String,
    /// The area modules `host.rs` includes.
    areas: String,
    fire: String,
    goal_view: String,
}

/// The hand-written sources, in deterministic order; `fire.rs` includes generated dispatch.
fn native_hook_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = root.join("crates/horizon-client/src/game/native_hooks");
    let mut paths = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|entry| entry.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().is_some_and(|name| name != "fire.rs")
    });
    paths.sort();
    Ok(paths)
}

/// Parse and validate the catalog and hook/helper signatures.
fn sources(root: &Path, fill: FillMode) -> Result<Sources, String> {
    #[allow(unused_mut)]
    let mut catalog = Catalog::load(root)?;
    let mut files = Vec::new();
    let mut methods = Vec::new();
    for path in native_hook_files(root)? {
        let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let file = syn::parse_file(&source).map_err(|e| e.to_string())?;
        let mut names = HashSet::new();
        for item in file.items {
            if let Item::Impl(item) = item {
                for method in impl_functions(item) {
                    names.insert(method.sig.ident.to_string());
                    methods.push(method);
                }
            }
        }
        files.push((path, source, names));
    }
    #[cfg(feature = "mixed-output-fixture")]
    mixed_output_fixture::append(&mut catalog, &mut methods)?;
    let dispatch::HookSources {
        traits,
        dispatch,
        part_traits,
        fills,
        fire,
        hook_ids,
        hook_constants,
        cancel_hooks,
        hook_count,
        ..
    } = dispatch::emit(
        root,
        catalog
            .hooks
            .iter()
            .filter(|hook| !dispatch::is_render(hook)),
        &methods,
        &catalog.draw_contexts,
    )?;
    if catalog.hooks.is_empty() {
        return Err(catalog::CatalogError::NoHooks.into());
    }
    let part = |name: &str| part_traits.get(name).map_or("", String::as_str);
    let code = format!(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse super::*;\n#[doc(hidden)]\n#[repr(usize)]\n#[derive(Clone, Copy, horizon_mod_api_macros::Wire)]\npub enum HookId {{ {hook_ids} }}\n#[doc(hidden)]\n#[allow(non_upper_case_globals)]\npub mod hook_ids {{ {hook_constants} }}\n#[doc(hidden)]\n#[allow(non_upper_case_globals)]\npub mod cancel_hooks {{ {cancel_hooks} }}\n/// How a trait mod's method listens, from its `#[listener]` attribute: a method without one has priority 0 and is not told of cancelled events.\n#[doc(hidden)]\n#[derive(Clone, Copy, horizon_mod_api_macros::Wire)]\npub struct Listener {{ pub hook: Option<HookId>, pub priority: i32, pub receive_cancelled: bool }}\npub(crate) const HOOK_COUNT: usize = {hook_count};\n/// The hooks a trait mod implements: one method per catalog hook, each documented from its `doc` field.\n/// Annotate the `impl` with `#[mod_api::horizon_mod]`, which subscribes the mod to just the methods it implements.\npub trait Hooks: std::any::Any + Send + Sync {{\n#[doc(hidden)] fn subscribed(&self) -> &'static [Option<HookId>];\n#[doc(hidden)] fn listeners(&self) -> &'static [Listener] {{ &[] }}\n/// Once, at startup: attach this mod's parts (`let shaft = r.block_key(\"mymod:shaft\"); r.block_part(shaft, Shaft)`).\nfn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {{ let _ = (r, cx); }}\n{traits}}}\n{PART_DOCS}pub trait BlockPart: Send + Sync + 'static {{\n#[doc(hidden)] fn block_subscribed(&self) -> &'static [Option<HookId>];\n/// Whether this part's `explosion_resistance` answer depends only on the state and the explosion, not on the position or the level around it: then a batched read (`LevelRef::explosion_resistances`) asks it once a state per call instead of once a cell. False unless the part says so; the answer stays a pure function of its inputs either way.\nfn explosion_resistance_is_pure(&self) -> bool {{ false }}\n{}}}\n/// A block's part that also answers for its block entity, and for the block like a [`BlockPart`].\npub trait BlockEntityPart: BlockPart {{\n#[doc(hidden)] fn block_entity_subscribed(&self) -> &'static [Option<HookId>];\n{}}}\n/// An item's part.\npub trait ItemPart: Send + Sync + 'static {{\n#[doc(hidden)] fn item_subscribed(&self) -> &'static [Option<HookId>];\n#[doc(hidden)] fn __prepare_equip(&self, stack: &ItemStack, kinds: &[&str], reads: &[EquipReadRow]) -> Result<Option<Box<dyn std::any::Any>>, &'static str> {{ let _ = (stack,kinds,reads); Ok(None) }}\n{}}}\n/// A mod menu type's part.\npub trait MenuPart: Send + Sync + 'static {{\n#[doc(hidden)] fn menu_subscribed(&self) -> &'static [Option<HookId>];\n{}}}\n/// A mod world fluid's part.\npub trait FluidPart: Send + Sync + 'static {{\n#[doc(hidden)] fn fluid_subscribed(&self) -> &'static [Option<HookId>];\n{}}}\n{ENTITY_PART_DOCS}pub trait EntityPart: Send + Sync + 'static {{\n#[doc(hidden)] fn entity_subscribed(&self) -> &'static [Option<HookId>] {{ &[] }}\n{}\n/// The mob's mod goals, added to its two selectors as `GoalSelector.addGoal`\n/// does in vanilla's `registerGoals`: fresh at every construction, so a goal's\n/// state is never saved. Branch on the mob with `goals.mob()`.\nfn register_goals(&self, goals: &mut GoalRegistrar<'_>) {{ let _ = goals; }}\n/// Native mods only: entity parts do not cross to wasm yet.\n/// The mob's mod brain behaviors, joined into the species' brain beside\n/// vanilla's (#824): built wherever the brain is built — its first tick and\n/// every rebuild — started and stopped in `Brain.tick`'s order, fresh state\n/// each build, nothing saved. Branch on the mob with `behaviors.mob()`.\nfn register_behaviors(&self, behaviors: &mut BehaviorRegistrar<'_>) {{ let _ = behaviors; }}\n/// How many mobs one spawn group of this mob's type may grow to (`Mob.getMaxSpawnClusterSize`), asked\n/// where the spawner has just added a mob to a group and compares the group's size with the limit:\n/// natural spawning's packs. `mob` is the mob just spawned, finalized and in\n/// no level yet, to read through `host`; `reason` and `spawner` say why and from what it spawned\n/// (`spawner` is none but for a spawner block's mobs); `vanilla` is the limit the species has. Answer\n/// `Some(limit)` to replace it and `None` to leave it to the next part and then to vanilla's. The\n/// first part to answer wins, in dispatch order, and the group stops growing once its size reaches\n/// the limit, so a limit of 0 or 1 ends it with the mob just spawned. The host is the read-only\n/// one: a listener runs on replayed ticks and must be a pure function of its inputs.\nfn spawn_cluster_size(&self, host: &dyn Host, mob: UnspawnedMobRef, reason: SpawnReason, spawner: Option<SpawnSource>, vanilla: u32) -> Option<u32> {{ let _ = (host, mob, reason, spawner, vanilla); None }}\n/// Whether `entity` can be sheared now with `shears` (NeoForge's `IShearable.isShearable`), asked\n/// where a player's shears (`player` is who holds them) or a dispenser's (`player` none) meet it,\n/// before the `player_shear_entity` or `block_shear_entity` hook. The first part to answer wins, in\n/// dispatch order: `Some(true)` shears it through [`EntityPart::on_sheared`] in place of the\n/// species' own shearing, `Some(false)` leaves it unsheared (a player's click then does nothing),\n/// and `None` leaves it to the next part and then to the species. Server-side.\nfn is_shearable(&self, host: &dyn Host, entity: EntityRef, player: Option<PlayerRef>, shears: &ItemStack) -> Option<bool> {{ let _ = (host, entity, player, shears); None }}\n/// Shears `entity` (NeoForge's `IShearable.onSheared`): called on the part whose\n/// [`EntityPart::is_shearable`] said `Some(true)`, once the shear hooks let it through. Change\n/// the entity and play its sound here, and return the drops: the game spawns each as\n/// `spawnShearedDrop` does (one block above the entity's feet, with a small random push), posts\n/// the shear game event and damages the shears. Server-side.\nfn on_sheared(&self, host: &mut dyn ServerHost, entity: EntityRef, player: Option<PlayerRef>, shears: &ItemStack) -> Vec<ItemStack> {{ let _ = (host, entity, player, shears); Vec::new() }}\n}}\n{dispatch}",
        part("BlockPart"),
        part("BlockEntityPart"),
        part("ItemPart"),
        part("MenuPart"),
        part("FluidPart"),
        part("EntityPart"),
    );
    let code = complete_hook_constants(code, "Hooks", "hook_ids", "HookId")?;
    let (host, goal_view) = host::generate(root)?;
    let areas = host::areas_source(root)?;
    // A new hook's placeholder body becomes its dispatch call.
    if !fills.is_empty() && matches!(fill, FillMode::Require) {
        let natives = fills
            .iter()
            .map(|fill| fill.native.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "{natives}: the let _ = (…); placeholder needs its dispatch call; run cargo run -p xtask -- mod-api"
        ));
    }
    for code in [&code, &host, &areas, &goal_view] {
        syn::parse_file(code).map_err(|e| format!("generated Rust: {e}"))?;
    }
    if matches!(fill, FillMode::Fill) {
        let mut outputs = Vec::new();
        for (path, source, names) in &files {
            let local = fills
                .iter()
                .filter(|fill| names.contains(&fill.native))
                .cloned()
                .collect::<Vec<_>>();
            if !local.is_empty() {
                outputs.push((path, filled_hooks(source, &local)?));
            }
        }
        for (path, output) in outputs {
            fs::write(path, output).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(Sources {
        hooks: code,
        host,
        areas,
        fire,
        goal_view,
    })
}

#[derive(Clone, Copy)]
enum FillMode {
    Fill,
    Require,
    Ignore,
}

mod build;
mod capability_providers;
pub mod crossing;
mod dispatch;
mod host_param;
mod index;
mod wire;
pub use wire::generate as generate_wire;
pub use wire::generate_coverage as generate_wire_coverage;

static GENERATED_ON_STDOUT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// For a tool that prints generated text on stdout (`xtask mod-api coverage`): the generator's
/// `cargo:rerun-if-changed` lines, meant for a build script's Cargo, are left out of it.
pub fn generated_on_stdout() {
    GENERATED_ON_STDOUT.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
thread_local! {
    /// The paths `rerun_if_changed` was given while a test holds the capture.
    static CAPTURED_RERUNS: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Tracks the stager's stamp, `.horizon-source-staged`, only in a staged copy. Cargo reruns a build
/// script on every build while a tracked path is missing, so tracking an absent stamp rebuilt
/// horizon-mod-points, horizon-client and every crate above them on each `cargo` invocation of an
/// ordinary checkout. A stamp never appears in a tree that was already built: staging writes it
/// first, into a fresh copy, and refuses a tree that already has one.
pub(crate) fn rerun_if_staged(root: &std::path::Path) {
    let stamp = root.join(".horizon-source-staged");
    if stamp.is_file() {
        rerun_if_changed(&stamp.display());
    }
}

/// Tells the build script's Cargo to rerun it when `path` changes.
pub(crate) fn rerun_if_changed(path: &impl std::fmt::Display) {
    #[cfg(test)]
    CAPTURED_RERUNS.with_borrow_mut(|capture| {
        if let Some(captured) = capture.as_mut() {
            captured.push(path.to_string());
        }
    });
    if !GENERATED_ON_STDOUT.load(std::sync::atomic::Ordering::Relaxed) {
        println!("cargo:rerun-if-changed={path}");
    }
}
pub use wire::generate_host as generate_wire_host;
pub mod functions;
pub mod internals;
pub mod internals_view;
pub mod loops;
pub mod notes;
pub mod patch;
pub mod points;
pub mod replace;
pub(crate) mod source;
pub mod staging;
pub mod surface;
pub use build::{
    generate, generate_engine_points, generate_points, generate_render, write_if_changed,
};
pub use notes::generate as generate_hook_notes;
mod staged;
pub use staged::{MODS_STAGE, staged_mods};

/// Generate the dispatch test fixture without preventing xtask from filling new placeholders.
#[doc(hidden)]
pub fn generate_test_hooks(root: &Path, out_dir: &Path) -> Result<(), String> {
    build::track(root, true)?;
    dispatch_fixtures::generate(root, out_dir)?;
    // These fixtures deliberately substitute dispatch-only stub values for API
    // types. Encoding is exercised by the real mod_api's Wire tests, not stubs.
    let code = sources(root, FillMode::Ignore)?
        .hooks
        .replace(", horizon_mod_api_macros::Wire", "")
        .replace("#[derive(horizon_mod_api_macros::Wire)]\n", "");
    fs::write(out_dir.join("mod_api_generated.rs"), code).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use quote::ToTokens;
    #[test]
    fn hook_points_come_from_the_catalog_and_its_fragments() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let points = super::hook_points(&root).unwrap();
        let frame = points
            .iter()
            .find(|p| p.name == "render_frame_pre")
            .unwrap();
        assert_eq!(frame.version, 2);
        let post = points
            .iter()
            .find(|p| p.name == "render_frame_post")
            .unwrap();
        assert_eq!(post.version, 2);
        assert_eq!(
            frame.target,
            "horizon_client::game::native_hooks::render_frame_pre::Game<'_>::native_render_frame_pre"
        );
        assert!(!frame.scoped);
        let scoped = points
            .iter()
            .find(|p| p.name == "pre_remove_side_effects")
            .unwrap();
        assert!(scoped.scoped);
        assert_eq!(scoped.version, 1);
    }

    #[test]
    #[ignore = "daily"]
    fn fragment_hooks_generate() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = super::Catalog::load(&root).unwrap();
        let hook = catalog
            .hooks
            .iter()
            .find(|h| h.name == "recipes_received")
            .unwrap();
        assert_eq!(hook.contract, super::Contract::Listen);
        let generated = super::sources(&root, super::FillMode::Require)
            .unwrap()
            .hooks;
        assert!(generated.contains("fn recipes_received("));
        assert!(generated.contains("/// Side: client."));
    }

    #[test]
    fn generated_locals_do_not_shadow_hook_arguments() {
        let mut used = ["mods", "mods_", "mod_hook", "replacement", "host"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(super::fresh_name("mods", &mut used), "mods__");
        assert_eq!(super::fresh_name("mod_hook", &mut used), "mod_hook_");
        assert_eq!(super::fresh_name("replacement", &mut used), "replacement_");
        assert_eq!(super::fresh_name("host", &mut used), "host_");
    }

    #[test]
    fn level_hosts_match_receiver_mutability() {
        use super::{Hook, HostAccess, Side};
        let level = Hook::sample(
            "horizon_client::game::GameLevel<'_>::native_test",
            Side::Server,
        );
        let game = Hook::sample("horizon_client::game::Game<'_>::native_test", Side::Both);
        use super::host_param::{HostParam, host_param};
        let param = |ty: &str, call| {
            Some(HostParam {
                ty: ty.into(),
                call,
            })
        };
        assert_eq!(
            host_param(&level, Some(false), false),
            param("&dyn LevelHost", "self")
        );
        assert_eq!(
            host_param(&level, Some(true), false),
            param("&mut dyn LevelRandomHost", "self")
        );
        let ticks = "&mut super::mod_api_host::LevelTicks { level: self, ticks }";
        assert_eq!(
            host_param(&level, Some(false), true),
            param("&mut dyn LevelTicksHost", ticks)
        );
        assert_eq!(
            host_param(&level, Some(true), true),
            param("&mut dyn LevelWriteHost", ticks)
        );
        assert_eq!(host_param(&level, None, false), None);
        let level_both = Hook {
            side: Side::Both,
            ..level.clone()
        };
        assert_eq!(
            host_param(&level_both, Some(false), false),
            param("LevelSide", "super::mod_api_host::level_side(self)")
        );
        for (mutable, ticks) in [(Some(true), false), (Some(false), true)] {
            assert_eq!(
                super::host_param::validate(&level_both, "test", true, mutable, ticks).unwrap_err(),
                "test: a both-side level hook takes the read kind until a client level seam exists"
            );
        }
        assert_eq!(
            host_param(&game, Some(true), false),
            param("HostSide", "super::execution_role::host_side(self)")
        );
        assert_eq!(
            super::host_param::validate(&game, "test", false, Some(false), false).unwrap_err(),
            "test: a both-side hook with a &self target takes host \"read\""
        );
        let read = Hook {
            host: Some(HostAccess::Read),
            ..game.clone()
        };
        assert_eq!(
            host_param(&read, Some(false), false),
            param("&dyn Host", "&*self")
        );
        let client = Hook {
            side: Side::Client,
            ..game.clone()
        };
        assert_eq!(
            host_param(&client, Some(false), false),
            param("&dyn ClientHost", "&*self")
        );
        assert_eq!(
            host_param(&client, Some(true), false),
            param("&mut dyn ClientHost", "self")
        );
        let client_read = Hook {
            side: Side::Client,
            ..read.clone()
        };
        assert_eq!(
            super::host_param::validate(&client_read, "test", false, Some(false), false)
                .unwrap_err(),
            "test: this entry takes no host marker"
        );
        assert_eq!(
            super::host_param::validate(&read, "test", true, Some(false), false).unwrap_err(),
            "test: this entry takes no host marker"
        );
        // A hit's level: another receiver than `GameLevel`, marked as a level host.
        let hit = Hook {
            host: Some(HostAccess::Level),
            ..Hook::sample(
                "horizon_client::game::native_hooks::HitLevel<'_>::native_test",
                Side::Server,
            )
        };
        assert!(super::host_param::is_level(&hit));
        assert_eq!(
            host_param(&hit, Some(true), false),
            param("&mut dyn LevelRandomHost", "self")
        );
        super::host_param::validate(&hit, "test", true, Some(true), false).unwrap();
        // On both sides a hit's level reads, and its host says which side it is.
        let hit_both = Hook {
            side: Side::Both,
            ..hit.clone()
        };
        super::host_param::validate(&hit_both, "test", true, Some(false), false).unwrap();
        assert_eq!(
            host_param(&hit_both, Some(false), false),
            param("LevelSide", "super::mod_api_host::level_side(self)")
        );
        assert_eq!(
            super::host_param::validate(&hit_both, "test", true, Some(false), true).unwrap_err(),
            "test: this entry takes no host marker"
        );
        for (hook, mutable) in [
            (hit.clone(), Some(false)),
            (
                Hook {
                    side: Side::Both,
                    ..hit.clone()
                },
                Some(true),
            ),
            (
                Hook {
                    host: Some(HostAccess::Level),
                    ..level.clone()
                },
                Some(true),
            ),
        ] {
            assert_eq!(
                super::host_param::validate(&hook, "test", true, mutable, false).unwrap_err(),
                "test: this entry takes no host marker"
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn only_a_game_hooks_placeholder_body_is_filled() {
        let fill = |body: &str, in_game: bool| {
            let method: syn::ItemFn = syn::parse_str(&format!(
                "fn native_h(&mut self, player: u128, cancel: &mut bool) {{ {body} }}"
            ))
            .unwrap();
            super::needs_fill(&method, "dispatch_h", in_game)
        };
        assert_eq!(fill("let _ = (player, cancel);", true), Ok(true));
        assert_eq!(
            fill("mod_api::dispatch_h(self, mods(), player, cancel);", true),
            Ok(false)
        );
        for (body, in_game) in [
            ("let _ = (player, cancel);", false),
            ("let _ = (cancel, player);", true),
            ("let _ = player;", true),
            ("let _ = (player, cancel); *cancel = true;", true),
            ("", true),
        ] {
            assert!(
                fill(body, in_game)
                    .unwrap_err()
                    .starts_with("missing dispatch_h call in native_h: "),
                "{body}"
            );
        }
        let one: syn::ItemFn =
            syn::parse_quote! { fn native_one(&self, from: &str) { let _ = from; } };
        assert_eq!(super::needs_fill(&one, "dispatch_one", true), Ok(true));
        let source = "impl Game<'_> {\n    fn native_other(&self, a: u32) {\n        let _ = a;\n    }\n    fn native_h(&mut self, player: u128, name: &str, cancel: &mut bool) {\n        let _ = (player, name, cancel);\n    }\n}\n";
        let filled = super::filled_hooks(
            source,
            &[super::Fill {
                native: "native_h".into(),
                call:
                    "mod_api::dispatch_h(self, mod_api::__private::mods(), player, name, cancel);"
                        .into(),
            }],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(filled).unwrap(),
            "impl Game<'_> {\n    fn native_other(&self, a: u32) {\n        let _ = a;\n    }\n    fn native_h(&mut self, player: u128, name: &str, cancel: &mut bool) {\n        mod_api::dispatch_h(self, mod_api::__private::mods(), player, name, cancel);\n    }\n}\n"
        );
    }

    /// `item_picked_up` taken back to how a new hook arrives (its body the
    /// placeholder) in a copy of the sources: the check reports it, any other
    /// body is left alone, a hook without its attribute fails by name, and the
    /// generator brings back the shipped file byte for byte with the generated
    /// code unchanged.
    #[test]
    #[ignore = "daily"]
    fn new_hook_placeholder_is_reported_then_filled() {
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            if from.is_dir() {
                for entry in std::fs::read_dir(from).unwrap() {
                    let entry = entry.unwrap();
                    copy(&entry.path(), &to.join(entry.file_name()));
                }
            } else {
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                std::fs::copy(from, to).unwrap();
            }
        }
        let real = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let root = real
            .join("target")
            .join(format!("mod-api-fill-{}", std::process::id()));
        let mut inputs: Vec<String> = [
            "crates/horizon-mod-vocab/src/hooks",
            "crates/horizon-mod-vocab/src/exports",
            "crates/horizon-mod-vocab/src/host",
            "crates/horizon-mod-vocab/src/parts",
            "crates/horizon-client/src/game.rs",
            "crates/horizon-client/src/game",
            "crates/horizon-mod-vocab/src/lib.rs",
        ]
        .map(str::to_owned)
        .into();
        for hook in super::Catalog::load(&real).unwrap().hooks {
            if let Some((krate, path)) = super::free_function_source(&hook.target) {
                let (module, _) = path.rsplit_once("::").unwrap();
                inputs.push(format!(
                    "crates/{krate}/src/{}.rs",
                    module.replace("::", "/")
                ));
            }
        }
        for input in &inputs {
            copy(&real.join(input), &root.join(input));
        }
        // The first hook and helper, by declaration order, and the file declaring each.
        let catalog = super::Catalog::load(&real).unwrap();
        let declaring = |name: &str| {
            std::fs::read_dir(root.join(super::catalog::DECLARATIONS))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    std::fs::read_to_string(path)
                        .unwrap()
                        .contains(&format!("fn {name}();"))
                })
                .unwrap()
        };
        let first = declaring(&catalog.hooks[0].name);
        let original = std::fs::read_to_string(&first).unwrap();
        let without_counterparts = super::sources(&root, super::FillMode::Require)
            .unwrap()
            .hooks;
        std::fs::write(
            &first,
            original.replacen(
                "#[hook(",
                "#[hook(fabric = [\"Example.ONE\", \"Example.TWO\"], paper = \"ExampleEvent\", ",
                1,
            ),
        )
        .unwrap();
        assert_eq!(
            super::sources(&root, super::FillMode::Require)
                .unwrap()
                .hooks,
            without_counterparts
        );
        std::fs::write(&first, original).unwrap();
        let hooks = root.join("crates/horizon-client/src/game/native_hooks/item_picked_up.rs");
        let shipped = std::fs::read_to_string(&hooks).unwrap();
        let call = "self.fire_item_picked_up(player, item, can_pickup);";
        assert_eq!(shipped.matches(call).count(), 1);
        let unfinished = shipped.replace(call, "let _ = (player, item);");
        std::fs::write(&hooks, &unfinished).unwrap();
        assert!(
            super::fill_hooks(&root)
                .unwrap_err()
                .starts_with("missing dispatch_item_picked_up call in native_item_picked_up: ")
        );
        assert_eq!(std::fs::read_to_string(&hooks).unwrap(), unfinished);
        let placeholder = shipped.replace(call, "let _ = (player, item, can_pickup);");
        std::fs::write(&hooks, &placeholder).unwrap();
        assert_eq!(
            super::sources(&root, super::FillMode::Require).unwrap_err(),
            "native_item_picked_up: the let _ = (…); placeholder needs its dispatch call; run cargo run -p xtask -- mod-api"
        );
        assert_eq!(std::fs::read_to_string(&hooks).unwrap(), placeholder);
        // A hook without its attribute fails by name.
        let declared = declaring("item_picked_up");
        let shipped_declarations = std::fs::read_to_string(&declared).unwrap();
        let declaration = shipped_declarations.find("fn item_picked_up();").unwrap();
        let attribute = shipped_declarations[..declaration]
            .rfind("#[hook(")
            .unwrap();
        std::fs::write(
            &declared,
            format!(
                "{}{}",
                &shipped_declarations[..attribute],
                &shipped_declarations[declaration..]
            ),
        )
        .unwrap();
        assert!(
            super::fill_hooks(&root)
                .unwrap_err()
                .contains("item_picked_up: needs a #[hook(...)] attribute")
        );
        std::fs::write(&declared, shipped_declarations).unwrap();
        let second = root.join("crates/horizon-client/src/game/native_hooks/client_tick_pre.rs");
        let second_shipped = std::fs::read_to_string(&second).unwrap();
        std::fs::write(
            &second,
            second_shipped.replace("self.fire_client_tick_pre();", "let _ = ();"),
        )
        .unwrap();
        let decoy = hooks.with_file_name("decoy.rs");
        let decoy_source = "impl Game<'_> { /// mentions fn native_item_picked_up(\n fn unrelated(&self) { let _ = self; } }\n";
        std::fs::write(&decoy, decoy_source).unwrap();
        super::fill_hooks(&root).unwrap();
        // The generator writes LF; a Windows checkout (core.autocrlf) has CRLF.
        let lf = |s: &str| s.replace("\r\n", "\n");
        assert_eq!(
            lf(&std::fs::read_to_string(&second).unwrap()),
            lf(&second_shipped)
        );
        assert_eq!(std::fs::read_to_string(decoy).unwrap(), decoy_source);
        assert_eq!(lf(&std::fs::read_to_string(&hooks).unwrap()), lf(&shipped));
        assert_eq!(
            super::sources(&root, super::FillMode::Require)
                .unwrap()
                .hooks,
            super::sources(&real, super::FillMode::Require)
                .unwrap()
                .hooks,
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn event_fields_name_their_lifetimes() {
        for (ty, field, scalar, a, b) in [
            (
                "& mut WorldDrawn < '_ >",
                "& 'a mut WorldDrawn < 'b >",
                false,
                true,
                true,
            ),
            ("& str", "& 'a str", false, true, false),
            (
                "& mut Vec < (& 'static str , u32) >",
                "& 'a mut Vec < (& 'static str , u32) >",
                false,
                true,
                false,
            ),
            (
                "Option < & ItemStack >",
                "Option < & 'a ItemStack >",
                false,
                true,
                false,
            ),
            ("u32", "u32", false, false, false),
            ("& mut f32", "f32", true, false, false),
            (
                "& mut Option < bool >",
                "& 'a mut Option < bool >",
                false,
                true,
                false,
            ),
        ] {
            let mut lifetimes = super::EventLifetimes::default();
            let (field_type, is_scalar) = lifetimes.field(&syn::parse_str(ty).unwrap());
            assert_eq!(field_type.to_token_stream().to_string(), field);
            assert_eq!((is_scalar, lifetimes.a, lifetimes.b), (scalar, a, b));
        }
    }

    #[test]
    fn library_crate_targets_resolve_to_their_source() {
        assert_eq!(
            super::free_function_source("horizon_client_lib::menu::native_menu_slot_rules"),
            Some(("horizon-client", "menu::native_menu_slot_rules".to_owned()))
        );
        assert_eq!(
            super::free_function_source("horizon_client_lib::breathing::native_living_breathe"),
            Some((
                "horizon-client",
                "breathing::native_living_breathe".to_owned()
            ))
        );
        assert_eq!(
            super::free_function_source(
                "horizon_client_render::sound_engine::hooks::native_play_sound"
            ),
            Some((
                "horizon-client-render",
                "sound_engine::hooks::native_play_sound".to_owned()
            ))
        );
        for target in [
            "horizon_client::menu::native_menu_slot_rules",
            "horizon_client::breathing::native_living_breathe",
        ] {
            assert_eq!(super::free_function_source(target), None);
        }
        assert_eq!(
            super::free_function_source("horizon_client_lib::crafting::native_ingredient"),
            Some(("horizon-client", "crafting::native_ingredient".to_owned()))
        );
        assert_eq!(
            super::free_function_source(
                "horizon_client::game::native_hooks::Game<'_>::native_zoom"
            ),
            None
        );
    }

    #[test]
    fn contracted_hook_requires_dispatch_call() {
        let missing: syn::ImplItemFn = syn::parse_quote! { fn native_zoom(&self) { unrelated(); } };
        let present: syn::ImplItemFn =
            syn::parse_quote! { fn native_zoom(&self) { mod_api::dispatch_zoom(); } };
        assert!(!super::has_call(&missing.block, "dispatch_zoom"));
        assert!(super::has_call(&present.block, "dispatch_zoom"));
        let generated: syn::ItemFn = syn::parse_quote! {
            fn native_zoom(&self) { self.fire_zoom(); }
        };
        assert_eq!(
            super::needs_fill(&generated, "dispatch_zoom", true),
            Ok(false)
        );
    }
}
