//! The surface export: `surface.json` ([`mod_surface::Surface`]) from what the build parses. Hooks
//! come from the catalog, with their arguments from the generated event types; trait roles,
//! sides and "Not on wasm" from the wire index; everything else from a walk of the public items
//! of `horizon-mod-api` and the vocabulary it re-exports.
pub mod behaviours;
mod expand;
pub mod notes;
mod rust;
#[cfg(test)]
mod tests;
mod walk;
mod wire_forms;

use crate::catalog::{self, Catalog};
use crate::index::{Found, Index, ItemId};
use mod_surface::shape::{ShapeError, Shapes};
use mod_surface::*;
use quote::ToTokens;
use rust::{Layouts, Paths, param_names, print};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use syn::visit_mut::VisitMut;
use syn::{Item, Visibility};
use walk::{attrs, compiled, hidden};
use wire_forms::WireForm;

/// The crates whose own `macro_rules!` calls the export expands: the API and the crates its
/// public items come from.
const API_CRATES: &[&str] = &[
    "mod_api",
    "horizon_mod_api",
    "horizon_core",
    "horizon_mod_runtime",
];

/// What the wire index says about the traits that cross the boundary, by trait path as the
/// index writes it below `mod_api` (`Hooks`, `item::menu::SlotView`,
/// `ResourceHandler<ItemResource>`); [`build`] resolves each to its trait.
#[derive(Default)]
pub(crate) struct Boundary {
    roles: Vec<(String, Role)>,
    sides: Vec<(String, Side)>,
    /// (trait, method, why it cannot cross).
    unbound: Vec<(String, String, String)>,
    /// (union trait, area trait that is part of it).
    unions: Vec<(String, String)>,
}

impl Boundary {
    fn from_wire(surface: &crate::wire::Surface<'_>) -> Self {
        let mut boundary = Self::default();
        for owner in crate::wire::imports() {
            boundary.roles.push(((*owner).to_owned(), Role::Call));
        }
        for owner in crate::wire::exports() {
            boundary.roles.push(((*owner).to_owned(), Role::Implement));
        }
        for (owner, area) in &surface.parts {
            boundary.unions.push(((*owner).to_owned(), area.clone()));
        }
        for unbound in &surface.unbound {
            boundary.unbound.push((
                unbound.owner.to_owned(),
                unbound.name.clone(),
                unbound.why.clone(),
            ));
        }
        boundary.sides.push(("ServerHost".into(), Side::Server));
        boundary.sides.push(("ClientHost".into(), Side::Client));
        for (owner, accessor, _) in crate::wire::REACHED {
            let side = if accessor.starts_with("server_host") {
                Side::Server
            } else if accessor.starts_with("client_host") {
                Side::Client
            } else {
                continue;
            };
            boundary.sides.push(((*owner).to_owned(), side));
        }
        boundary
    }
}

/// [`Boundary`] resolved to the traits themselves.
#[derive(Default)]
struct Roles {
    role: HashMap<ItemId, Role>,
    side: HashMap<ItemId, Side>,
    unbound: HashMap<(ItemId, String), String>,
    /// An area trait → the union trait it is part of.
    union_of: HashMap<ItemId, ItemId>,
}

impl Roles {
    fn resolve(index: &mut Index<'_>, boundary: &Boundary, notes: &mut Vec<String>) -> Self {
        let mut roles = Self::default();
        let Ok(api) = index.module("mod_api", &[]) else {
            return roles;
        };
        let mut find = |owner: &str, notes: &mut Vec<String>| {
            let bare = owner.split('<').next().unwrap_or(owner);
            let segments: Vec<String> = bare.split("::").map(str::to_owned).collect();
            match index.resolve_path(api, &segments) {
                Ok(Found::Item(id)) => Some(id),
                _ => {
                    notes.push(format!(
                        "boundary trait {owner}: not found in mod_api, its role is not recorded"
                    ));
                    None
                }
            }
        };
        for (owner, role) in &boundary.roles {
            if let Some(id) = find(owner, notes) {
                roles.role.insert(id, *role);
            }
        }
        for (owner, side) in &boundary.sides {
            if let Some(id) = find(owner, notes) {
                roles.side.insert(id, *side);
            }
        }
        for (owner, area) in &boundary.unions {
            if let (Some(owner), Some(area)) = (find(owner, notes), find(area, notes)) {
                roles.union_of.insert(area, owner);
            }
        }
        for (owner, method, why) in &boundary.unbound {
            if let Some(id) = find(owner, notes) {
                roles.unbound.insert((id, method.clone()), why.clone());
            }
        }
        roles
    }

    fn find<'r, T>(&'r self, map: &'r HashMap<ItemId, T>, id: ItemId) -> Option<&'r T> {
        map.get(&id)
            .or_else(|| self.union_of.get(&id).and_then(|union| map.get(union)))
    }

    fn role(&self, id: ItemId) -> Role {
        self.find(&self.role, id).copied().unwrap_or(Role::None)
    }

    fn side(&self, id: ItemId) -> Side {
        self.find(&self.side, id).copied().unwrap_or(Side::Both)
    }

    fn unbound(&self, id: ItemId, method: &str) -> Option<&String> {
        self.unbound.get(&(id, method.to_owned())).or_else(|| {
            let union = self.union_of.get(&id)?;
            self.unbound.get(&(*union, method.to_owned()))
        })
    }
}

/// The export, with what it could not classify.
pub struct Export {
    pub surface: Surface,
    /// One line per public item or declaration the export has no record kind for, and the
    /// counts of what it skips by rule.
    pub notes: Vec<String>,
    /// The calls and core a wasm mod is checked by when it loads; `None` for an export of
    /// declarations alone.
    pub boundary: Option<mod_surface::boundary::Boundary>,
}

impl Export {
    /// How many records of each kind, members included.
    pub fn counts(&self) -> Vec<(&'static str, usize)> {
        let s = &self.surface;
        let form = |name: &str| {
            s.types
                .iter()
                .filter(|t| {
                    t.contract
                        .wire_form
                        .as_deref()
                        .is_some_and(|f| f.split(':').next() == Some(name))
                })
                .count()
        };
        vec![
            ("hooks", s.hooks.len()),
            ("traits", s.traits.len()),
            ("methods", s.traits.iter().map(|t| t.methods.len()).sum()),
            (
                "associated",
                s.traits.iter().map(|t| t.associated.len()).sum(),
            ),
            ("handles", s.handles.len()),
            ("functions", s.functions.len()),
            ("macros", s.macros.len()),
            ("types", s.types.len()),
            ("fields", s.types.iter().map(|t| t.fields.len()).sum()),
            ("variants", s.types.iter().map(|t| t.variants.len()).sum()),
            ("parts", s.parts.len()),
            ("capabilities", s.capabilities.len()),
            ("behaviours", s.behaviours.len()),
            ("wire", s.types.iter().filter(|t| t.contract.wire).count()),
            ("wire_derive", form("derive")),
            ("wire_fields", form("fields")),
            ("wire_via", form("via")),
            ("wire_manual", form("manual")),
            (
                "wire_without_shape",
                s.types
                    .iter()
                    .filter(|t| t.contract.wire && t.shape.is_none())
                    .count(),
            ),
        ]
    }
}

/// Exports the surface of the source tree at `root`.
pub fn export(root: &Path) -> Result<Export, String> {
    // The wire index's resolver recurses through the API's types: give it a deep stack.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn_scoped(scope, || export_here(root))
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|_| "the surface export panicked".to_string())?
    })
}

fn export_here(root: &Path) -> Result<Export, String> {
    let catalog = Catalog::load(root).map_err(|e| e.to_string())?;
    let generated = crate::sources(root, crate::FillMode::Require)?;
    let included = [
        (crate::host::AREAS_OUTPUT, generated.areas.as_str()),
        ("mod_api_generated.rs", generated.hooks.as_str()),
        ("mod_api_generated_host.rs", generated.host.as_str()),
    ];
    let providers = crate::capability_providers::generate(root, &included)?.0;
    let mut with_providers = included.to_vec();
    with_providers.push(("capability_providers_generated.rs", providers.as_str()));
    // A listed boundary trait an older tree lacks is noted by `Roles::resolve`, not refused.
    let wire = crate::wire::surface_reading(root, &with_providers, Some(&mut Vec::new()))?;
    let boundary = Boundary::from_wire(&wire);
    let calls = crate::wire::boundary(root, &wire)?;
    let mut index = wire.types.index;
    build(root, &mut index, &catalog.hooks, &boundary, Some(calls))
}

/// The `///` text of `attrs`, one line each, a leading space dropped.
fn doc(attrs: &[syn::Attribute]) -> String {
    let lines: Vec<String> = attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(pair) if pair.path.is_ident("doc") => match &pair.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(text),
                    ..
                }) => Some(text.value()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    lines
        .iter()
        .map(|line| line.strip_prefix(' ').unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// A doc's `Not on wasm: <why>`, to the end of its paragraph.
fn not_on_wasm(attrs: &[syn::Attribute]) -> Option<String> {
    let text = doc(attrs);
    let mut lines = text.lines().map(str::trim);
    let first = lines.find_map(|line| line.strip_prefix("Not on wasm: "))?;
    let why: Vec<&str> = std::iter::once(first)
        .chain(lines.take_while(|line| !line.is_empty()))
        .collect();
    Some(why.join(" ").trim().to_owned()).filter(|why| why.len() > 1)
}

fn has_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

/// The traits of `attrs`' `#[derive(…)]` lists, by last path segment, sorted.
pub(super) fn derives(attrs: &[syn::Attribute]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("derive")) {
        if let Ok(paths) = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
        ) {
            for path in paths {
                if let Some(last) = path.segments.last() {
                    out.insert(last.ident.to_string());
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Normalises types and signatures written in one module.
struct Writer<'p, 'a> {
    index: &'p mut Index<'a>,
    public: &'p HashMap<ItemId, Vec<String>>,
}

impl<'a> Writer<'_, 'a> {
    fn paths(&mut self, module: usize, params: &[String]) -> Paths<'_, 'a> {
        Paths {
            index: self.index,
            public: self.public,
            module,
            params: params.to_vec(),
            refs: BTreeSet::new(),
        }
    }

    fn ty(&mut self, module: usize, params: &[String], ty: &syn::Type) -> (String, Vec<String>) {
        let mut ty = ty.clone();
        let mut paths = self.paths(module, params);
        paths.visit_type_mut(&mut ty);
        (
            print(ty.to_token_stream()),
            paths.refs.into_iter().collect(),
        )
    }

    fn path(
        &mut self,
        module: usize,
        params: &[String],
        path: &syn::Path,
    ) -> (String, Vec<String>) {
        let ty = syn::Type::Path(syn::TypePath {
            qself: None,
            path: path.clone(),
        });
        self.ty(module, params, &ty)
    }

    fn signature(
        &mut self,
        module: usize,
        params: &[String],
        sig: &syn::Signature,
    ) -> (String, Vec<String>) {
        let mut sig = sig.clone();
        for input in &mut sig.inputs {
            match input {
                syn::FnArg::Receiver(receiver) => receiver.attrs.clear(),
                syn::FnArg::Typed(arg) => {
                    arg.attrs.clear();
                    // A parameter's name is not part of what a caller relies on.
                    *arg.pat = syn::parse_quote!(_);
                }
            }
        }
        let mut params = params.to_vec();
        params.extend(param_names(&sig.generics));
        let mut paths = self.paths(module, &params);
        paths.visit_signature_mut(&mut sig);
        (
            print(sig.to_token_stream()),
            paths.refs.into_iter().collect(),
        )
    }

    fn generics(&mut self, module: usize, generics: &syn::Generics) -> (String, Vec<String>) {
        let mut generics = generics.clone();
        let params = param_names(&generics);
        let mut paths = self.paths(module, &params);
        paths.visit_generics_mut(&mut generics);
        let mut text = print(generics.to_token_stream());
        if let Some(clause) = &generics.where_clause {
            text.push(' ');
            text.push_str(&print(clause.to_token_stream()));
        }
        (text.trim().to_owned(), paths.refs.into_iter().collect())
    }

    /// A trait item written without its default (`type Entry: DataMapEntry`,
    /// `const MAX: u32`), and its references.
    fn associated(
        &mut self,
        module: usize,
        params: &[String],
        item: &syn::TraitItem,
    ) -> Option<(String, AssociatedKind, bool, Vec<String>)> {
        let (mut written, kind, required) = match item {
            syn::TraitItem::Type(ty) => {
                let mut ty = ty.clone();
                let required = ty.default.is_none();
                ty.default = None;
                ty.attrs.clear();
                (syn::TraitItem::Type(ty), AssociatedKind::Type, required)
            }
            syn::TraitItem::Const(constant) => {
                let mut constant = constant.clone();
                let required = constant.default.is_none();
                constant.default = None;
                constant.attrs.clear();
                (
                    syn::TraitItem::Const(constant),
                    AssociatedKind::Const,
                    required,
                )
            }
            _ => return None,
        };
        let mut paths = self.paths(module, params);
        paths.visit_trait_item_mut(&mut written);
        let refs = paths.refs.into_iter().collect();
        let text = print(written.to_token_stream());
        Some((
            text.trim_end_matches(';').trim().to_owned(),
            kind,
            required,
            refs,
        ))
    }
}

fn merge(into: &mut Vec<String>, refs: Vec<String>) {
    into.extend(refs);
    into.sort();
    into.dedup();
}

/// An item's public inherent members: methods and consts of `impl Type` and `impl dyn Trait`
/// blocks, with the block's module, generic parameter names (and `Self`) and generics text.
struct Inherent {
    module: usize,
    params: Vec<String>,
    generics: syn::Generics,
    item: syn::ImplItem,
}

/// What [`inherent_items`] and [`trait_impls`] found.
struct Impls {
    inherent: HashMap<ItemId, Vec<Inherent>>,
    /// The traits implemented for each nameable type, by trait path as written in its module.
    traits: HashMap<ItemId, Vec<(usize, Vec<String>, syn::Path)>>,
    /// Impl blocks whose self type the export does not record, by kind.
    skipped: BTreeMap<&'static str, usize>,
    /// Impl blocks and inherent members compiled out of a default native build.
    gated: Vec<String>,
}

fn module_name(index: &Index<'_>, module: usize) -> String {
    let m = &index.modules[module];
    std::iter::once(m.krate)
        .chain(m.path.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("::")
}

fn impls(
    index: &mut Index<'_>,
    modules: &[usize],
    nameable: &HashMap<ItemId, Vec<String>>,
) -> Impls {
    let mut out = Impls {
        inherent: HashMap::new(),
        traits: HashMap::new(),
        skipped: BTreeMap::new(),
        gated: Vec::new(),
    };
    for &module in modules {
        let module_items = index.modules[module].items.clone();
        for item in module_items {
            let Item::Impl(block) = item else { continue };
            if !compiled(&block.attrs) {
                out.gated.push(format!(
                    "impl {} in {}: compiled out of a default native build ({}), not recorded",
                    print(block.self_ty.to_token_stream()),
                    module_name(index, module),
                    walk::cfg_text(&block.attrs)
                ));
                continue;
            }
            let path = match wire_forms::ungroup(&block.self_ty) {
                syn::Type::Path(self_ty) if self_ty.qself.is_none() => self_ty.path.clone(),
                // `impl dyn Trait + '_`: inherent items of the trait object.
                syn::Type::TraitObject(object) if block.trait_.is_none() => {
                    match object.bounds.iter().find_map(|bound| match bound {
                        syn::TypeParamBound::Trait(t) => Some(t.path.clone()),
                        _ => None,
                    }) {
                        Some(path) => path,
                        None => continue,
                    }
                }
                other => {
                    let kind = match other {
                        syn::Type::Reference(_) => "a reference",
                        syn::Type::TraitObject(_) => "a trait object (trait impl)",
                        syn::Type::Tuple(_) => "a tuple",
                        syn::Type::Slice(_) | syn::Type::Array(_) => "a slice or array",
                        syn::Type::Path(_) => "a qualified path",
                        _ => "another type form",
                    };
                    *out.skipped.entry(kind).or_default() += 1;
                    continue;
                }
            };
            let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
            let Ok(Found::Item(owner)) = index.resolve_path(module, &segments) else {
                continue;
            };
            if !nameable.contains_key(&owner) {
                continue;
            }
            let mut params = param_names(&block.generics);
            if let Some((_, trait_path, _)) = &block.trait_ {
                out.traits
                    .entry(owner)
                    .or_default()
                    .push((module, params, trait_path.clone()));
                continue;
            }
            params.push("Self".into());
            for impl_item in &block.items {
                let (vis, attrs) = match impl_item {
                    syn::ImplItem::Fn(f) => (&f.vis, &f.attrs),
                    syn::ImplItem::Const(c) => (&c.vis, &c.attrs),
                    _ => continue,
                };
                if matches!(vis, Visibility::Public(_)) && !compiled(attrs) && !hidden(attrs) {
                    let name = match impl_item {
                        syn::ImplItem::Fn(f) => f.sig.ident.to_string(),
                        syn::ImplItem::Const(c) => c.ident.to_string(),
                        _ => String::new(),
                    };
                    out.gated.push(format!(
                        "{}::{name}: compiled out of a default native build ({}), not recorded",
                        print(block.self_ty.to_token_stream()),
                        walk::cfg_text(attrs)
                    ));
                    continue;
                }
                if matches!(vis, Visibility::Public(_)) && compiled(attrs) && !hidden(attrs) {
                    out.inherent.entry(owner).or_default().push(Inherent {
                        module,
                        params: params.clone(),
                        generics: block.generics.clone(),
                        item: impl_item.clone(),
                    });
                }
            }
        }
    }
    out
}

/// A type before its shape is known.
struct Pending {
    id: String,
    item: ItemId,
    doc: String,
    paths: Vec<String>,
    impls: Vec<String>,
    /// The API items its impls' trait paths name (`From<Input>` names `Input`): found through the
    /// type, so they get records, but outside its contract and hash.
    impl_refs: Vec<String>,
    fields_hash: Option<String>,
    contract: TypeContract,
    fields: Vec<Field>,
    variants: Vec<Variant>,
}

/// The surface of the API whose modules `index` reads, with `hooks` from the catalog.
pub(crate) fn build(
    root: &Path,
    index: &mut Index<'_>,
    hooks: &[catalog::Hook],
    boundary: &Boundary,
    mut calls: Option<mod_surface::boundary::Boundary>,
) -> Result<Export, String> {
    // The facade crate is indexed for this walk only.
    index.add_crate("horizon_mod_api", "crates/horizon-mod-api/src");
    // The API crates' own macro calls, expanded before anything reads their modules.
    let mut notes = Vec::new();
    let mut expanded = Vec::new();
    {
        let mut walker = walk::Walker::new(index);
        for krate in API_CRATES {
            if let Ok(found) = walker.all_modules(krate) {
                expanded.extend(found);
            }
        }
    }
    notes.extend(expand::expand_modules(index, &expanded));
    let mut walker = walk::Walker::new(index);
    let public = walker.public()?;
    notes.extend(public.notes.iter().cloned());
    // Inherent items and impls live in any module of the crate that defines the type.
    let mut crates: BTreeSet<&'static str> = BTreeSet::from(["mod_api", "horizon_mod_api"]);
    for id in public.items.keys() {
        crates.insert(walker.index.modules[id.module].krate);
    }
    let mut modules = Vec::new();
    for krate in crates {
        modules.extend(walker.all_modules(krate)?);
    }
    notes.extend(walker.take_notes());
    let roles = Roles::resolve(index, boundary, &mut notes);
    // Every name a mod can write; what is behind `#[doc(hidden)]` is recorded once a recorded
    // signature names it.
    let mut nameable = public.items.clone();
    nameable.extend(public.hidden.iter().map(|(id, path)| (*id, path.clone())));
    let by_path: HashMap<String, ItemId> = nameable
        .iter()
        .map(|(id, path)| (path.join("::"), *id))
        .collect();
    let forms = wire_forms::wire_forms(index, &modules);
    let found = impls(index, &modules, &nameable);
    notes.extend(found.gated.iter().cloned());
    for (kind, count) in &found.skipped {
        notes.push(format!(
            "{count} impl blocks on {kind}: no record kind for their items"
        ));
    }
    let mut writer = Writer {
        index,
        public: &nameable,
    };
    let mut queue: Vec<(ItemId, Vec<String>)> = public
        .items
        .iter()
        .map(|(id, path)| (*id, path.clone()))
        .collect();
    queue.sort_by(|a, b| b.1.cmp(&a.1));
    // A hook's event type is part of the surface even when no public signature names it (a
    // scoped hook's event).
    let events: BTreeSet<String> = hooks
        .iter()
        .map(|hook| format!("{}Event", crate::hook_id(&hook.name)))
        .collect();
    let mut hidden_events: Vec<(ItemId, Vec<String>)> = public
        .hidden
        .iter()
        .filter(|(_, path)| path.last().is_some_and(|name| events.contains(name)))
        .map(|(id, path)| (*id, path.clone()))
        .collect();
    hidden_events.sort_by(|a, b| b.1.cmp(&a.1));
    queue.extend(hidden_events);
    let mut recorded: std::collections::HashSet<ItemId> = queue.iter().map(|(id, _)| *id).collect();
    let mut traits = Vec::new();
    let mut functions = Vec::new();
    let mut pending = Vec::new();
    let mut capabilities = Vec::new();
    let mut handles: BTreeMap<String, Function> = BTreeMap::new();
    while let Some((id, path)) = queue.pop() {
        let path = path.join("::");
        let paths = public.paths.get(&id).cloned().unwrap_or_default();
        let module = id.module;
        let item = writer.index.item(id).clone();
        let mut named: Vec<String> = Vec::new();
        let owner = match &item {
            Item::Trait(_) => format!("trait:{path}"),
            _ => format!("type:{path}"),
        };
        match &item {
            Item::Struct(_) | Item::Enum(_) | Item::Union(_) | Item::Type(_) => {
                let impls = found.traits.get(&id).map(Vec::as_slice).unwrap_or_default();
                let mut record = type_record(&mut writer, id, &path, &item, impls, &forms);
                record.paths = paths;
                named.extend(record.contract.refs.iter().cloned());
                named.extend(record.impl_refs.iter().cloned());
                for field in &record.fields {
                    named.extend(field.contract.refs.iter().cloned());
                }
                for variant in &record.variants {
                    named.extend(variant.contract.refs.iter().cloned());
                }
                pending.push(record);
            }
            Item::Trait(item) => {
                let mut record = trait_record(&mut writer, &mut notes, &roles, id, &path, item);
                record.paths = paths;
                named.extend(record.contract.refs.iter().cloned());
                for method in &record.methods {
                    named.extend(method.contract.refs.iter().cloned());
                }
                for associated in &record.associated {
                    named.extend(associated.contract.refs.iter().cloned());
                }
                traits.push(record);
            }
            Item::Fn(function) => {
                let (signature, refs) = writer.signature(module, &[], &function.sig);
                named.extend(refs.iter().cloned());
                let mut record = Function::new(
                    format!("fn:{path}"),
                    doc(&function.attrs),
                    FunctionContract {
                        signature,
                        impl_generics: String::new(),
                        wasm: not_on_wasm(&function.attrs).is_none(),
                        refs,
                    },
                );
                record.wasm_reason = not_on_wasm(&function.attrs);
                record.paths = paths;
                functions.push(record);
            }
            Item::Const(constant) => match capability_key(&mut writer, module, &path, constant) {
                Some(capability) => {
                    named.extend(capability.contract.refs.iter().cloned());
                    capabilities.push(capability);
                }
                None => notes.push(format!("constant {path}: no record kind for constants")),
            },
            Item::Static(_) => notes.push(format!("static {path}: no record kind for statics")),
            _ => notes.push(format!("{path}: not a kind the surface records")),
        }
        for member in found.inherent.get(&id).into_iter().flatten() {
            let (name, attrs, written) = match &member.item {
                syn::ImplItem::Fn(method) => (
                    method.sig.ident.to_string(),
                    &method.attrs,
                    writer.signature(member.module, &member.params, &method.sig),
                ),
                syn::ImplItem::Const(constant) => {
                    let (ty, refs) = writer.ty(member.module, &member.params, &constant.ty);
                    (
                        constant.ident.to_string(),
                        &constant.attrs,
                        (format!("const {}: {ty}", constant.ident), refs),
                    )
                }
                _ => continue,
            };
            let handle_id = format!("handle:{path}::{name}");
            if handles.contains_key(&handle_id) {
                notes.push(format!(
                    "{handle_id}: a second inherent item of that name (another impl block), not recorded"
                ));
                continue;
            }
            let (impl_generics, generic_refs) = writer.generics(member.module, &member.generics);
            let (signature, mut refs) = written;
            merge(&mut refs, generic_refs);
            merge(&mut refs, vec![owner.clone()]);
            named.extend(refs.iter().cloned());
            let mut record = Function::new(
                handle_id.clone(),
                doc(attrs),
                FunctionContract {
                    signature,
                    impl_generics,
                    wasm: not_on_wasm(attrs).is_none(),
                    refs,
                },
            );
            record.wasm_reason = not_on_wasm(attrs);
            handles.insert(handle_id, record);
        }
        for reference in named {
            let Some((_, target)) = reference.split_once(':') else {
                continue;
            };
            if let Some(next) = by_path.get(target)
                && recorded.insert(*next)
            {
                queue.push((*next, nameable[next].clone()));
            }
        }
    }

    let macros = macros(root, writer.index, &public.macros, &mut notes)?;

    // Shapes, over every type the crossing types contain.
    let mut layouts = Layouts::new(&forms, &nameable);
    let keys: Vec<Option<String>> = pending
        .iter()
        .map(|record| {
            (record.contract.wire && record.contract.kind != TypeKind::Alias)
                .then(|| layouts.key(writer.index, record.item))
        })
        .collect();
    // The owned types the calls and the core reach, hidden ones too, by the path the glue
    // names them by (`mod_api::PackageInfo`).
    let owned_keys: HashMap<String, String> = calls
        .iter()
        .flat_map(|b| {
            b.core_owned
                .iter()
                .chain(b.calls.iter().flat_map(|c| &c.owned))
        })
        .filter_map(|owned| {
            let path = owned.path.split('<').next()?.trim();
            let path = path.strip_prefix("mod_api::").unwrap_or(path);
            // A type's record is at its defining path; the glue names it by any public one.
            let id = by_path.get(path).copied().or_else(|| {
                pending
                    .iter()
                    .find(|record| record.paths.iter().any(|p| p == path))
                    .map(|record| record.item)
            })?;
            Some((owned.path.clone(), layouts.key(writer.index, id)))
        })
        .collect();
    layouts.finish(writer.index);
    let mut shapes = Shapes::new(&layouts.table);
    let mut types = Vec::new();
    for (record, key) in pending.into_iter().zip(keys) {
        let shape = match key {
            Some(key) => match shapes.shape(&key) {
                Ok(shape) => Some(shape),
                Err(ShapeError::Undefined(missing)) => {
                    notes.push(format!(
                        "{}: no shape, because {} crosses by a hand-written Wire impl that declares no layout, or does not cross",
                        record.id,
                        layouts.named(&missing)
                    ));
                    None
                }
                Err(error) => {
                    notes.push(format!("{}: no shape: {error}", record.id));
                    None
                }
            },
            None => None,
        };
        let mut built = Type::new(
            record.id,
            record.doc,
            shape,
            record.contract,
            record.fields,
            record.variants,
        );
        built.paths = record.paths;
        built.impls = record.impls;
        built.fields_hash = record.fields_hash;
        types.push(built);
    }

    if let Some(calls) = &mut calls {
        let mut fill = |owned: &mut mod_surface::boundary::Owned| {
            owned.shape = owned_keys
                .get(&owned.path)
                .and_then(|key| shapes.shape(key).ok());
        };
        calls.core_owned.iter_mut().for_each(&mut fill);
        calls
            .calls
            .iter_mut()
            .flat_map(|c| c.owned.iter_mut())
            .for_each(&mut fill);
    }
    let hooks = hook_records(hooks, &types, &mut notes);
    let parts = part_records(&hooks, &traits, &mut notes);
    capabilities.extend(manifest_capabilities(root, &mut notes));
    let behaviours = behaviours::read(root, &root.join("crates/horizon-mod-vocab/behaviours"))
        .map_err(|e| e.to_string())?;
    notes.sort();
    notes.dedup();
    Ok(Export {
        surface: Surface::new(
            hooks,
            traits,
            handles.into_values().collect(),
            functions,
            macros,
            types,
            parts,
            capabilities,
        )
        .with_behaviours(behaviours),
        notes,
        boundary: calls,
    })
}

fn trait_record(
    writer: &mut Writer<'_, '_>,
    notes: &mut Vec<String>,
    roles: &Roles,
    id: ItemId,
    path: &str,
    item: &syn::ItemTrait,
) -> Trait {
    let module = id.module;
    let owner = format!("trait:{path}");
    let role = roles.role(id);
    let (generics, mut refs) = writer.generics(module, &item.generics);
    let mut params = param_names(&item.generics);
    params.push("Self".into());
    let mut bounds = Vec::new();
    for bound in &item.supertraits {
        let mut bound = bound.clone();
        let mut paths = writer.paths(module, &params);
        paths.visit_type_param_bound_mut(&mut bound);
        let found: Vec<String> = paths.refs.into_iter().collect();
        merge(&mut refs, found);
        bounds.push(print(bound.to_token_stream()));
    }
    let mut methods = Vec::new();
    let mut associated = Vec::new();
    let mut required = Vec::new();
    for member in &item.items {
        let attrs = match member {
            syn::TraitItem::Fn(f) => &f.attrs,
            syn::TraitItem::Type(t) => &t.attrs,
            syn::TraitItem::Const(c) => &c.attrs,
            _ => continue,
        };
        if hidden(attrs) {
            continue;
        }
        if !compiled(attrs) {
            let name = match member {
                syn::TraitItem::Fn(f) => f.sig.ident.to_string(),
                syn::TraitItem::Type(t) => t.ident.to_string(),
                syn::TraitItem::Const(c) => c.ident.to_string(),
                _ => String::new(),
            };
            notes.push(format!(
                "{owner} member {name}: compiled out of a default native build ({}), not recorded",
                walk::cfg_text(attrs)
            ));
            continue;
        }
        if let Some((signature, kind, is_required, mut member_refs)) =
            writer.associated(module, &params, member)
        {
            let name = match member {
                syn::TraitItem::Type(t) => t.ident.to_string(),
                syn::TraitItem::Const(c) => c.ident.to_string(),
                _ => unreachable!("associated items only"),
            };
            if is_required && role != Role::Call {
                required.push(Required {
                    name: name.clone(),
                    signature: signature.clone(),
                });
            }
            merge(&mut member_refs, vec![owner.clone()]);
            associated.push(Associated::new(
                format!("assoc:{path}::{name}"),
                doc(attrs),
                AssociatedContract {
                    kind,
                    signature,
                    required: is_required,
                    refs: member_refs,
                },
            ));
            continue;
        }
        let syn::TraitItem::Fn(method) = member else {
            continue;
        };
        let method_name = method.sig.ident.to_string();
        let (signature, mut method_refs) = writer.signature(module, &params, &method.sig);
        merge(&mut method_refs, vec![owner.clone()]);
        let reason = match not_on_wasm(&method.attrs) {
            Some(why) => Some(why),
            None if role != Role::None => roles.unbound(id, &method_name).cloned(),
            None => None,
        };
        let is_required = method.default.is_none();
        if is_required && role != Role::Call {
            required.push(Required {
                name: method_name.clone(),
                signature: signature.clone(),
            });
        }
        let mut record = Method::new(
            format!("method:{path}::{method_name}"),
            doc(&method.attrs),
            MethodContract {
                signature,
                required: is_required,
                wasm: reason.is_none(),
                refs: method_refs,
            },
        );
        record.wasm_reason = reason;
        methods.push(record);
    }
    Trait::new(
        owner,
        doc(&item.attrs),
        TraitContract {
            role,
            side: roles.side(id),
            generics,
            bounds,
            required,
            refs,
        },
        methods,
        associated,
    )
}

fn type_record(
    writer: &mut Writer<'_, '_>,
    item_id: ItemId,
    path: &str,
    item: &Item,
    trait_impls: &[(usize, Vec<String>, syn::Path)],
    forms: &HashMap<ItemId, WireForm>,
) -> Pending {
    let module = item_id.module;
    let id = format!("type:{path}");
    let attrs = attrs(item).to_vec();
    let non_exhaustive = has_attr(&attrs, "non_exhaustive");
    let derives = derives(&attrs);
    let mut fields = Vec::new();
    let mut variants = Vec::new();
    let mut members = Vec::new();
    let mut target = None;
    let empty = syn::Generics::default();
    let item_generics = match item {
        Item::Struct(s) => &s.generics,
        Item::Enum(e) => &e.generics,
        Item::Union(u) => &u.generics,
        Item::Type(t) => &t.generics,
        _ => &empty,
    };
    let (generics, mut contract_refs) = writer.generics(module, item_generics);
    let params = param_names(item_generics);
    let owner = id.clone();
    // Every public field a literal must name, `#[doc(hidden)]` ones included.
    let mut literal_fields: Vec<(String, String)> = Vec::new();
    let mut field_records = |writer: &mut Writer<'_, '_>, list: &syn::Fields| -> bool {
        let mut all_public = true;
        for (position, field) in list.iter().enumerate() {
            let (ty, mut refs) = writer.ty(module, &params, &field.ty);
            let name = field
                .ident
                .as_ref()
                .map_or_else(|| position.to_string(), ToString::to_string);
            if !matches!(field.vis, Visibility::Public(_)) {
                all_public = false;
                continue;
            }
            literal_fields.push((name.clone(), ty.clone()));
            if hidden(&field.attrs) {
                continue;
            }
            merge(&mut refs, vec![owner.clone()]);
            fields.push(Field::new(
                format!("{id}.{name}"),
                doc(&field.attrs),
                FieldContract { ty, refs },
            ));
        }
        all_public
    };
    let (kind, constructible) = match item {
        Item::Struct(s) => {
            let kind = match &s.fields {
                syn::Fields::Named(_) => TypeKind::Struct,
                syn::Fields::Unnamed(_) => TypeKind::TupleStruct,
                syn::Fields::Unit => TypeKind::UnitStruct,
            };
            // A struct's field set is its `fields_hash`, outside its own hash.
            let all_public = field_records(writer, &s.fields);
            (kind, all_public && !non_exhaustive)
        }
        Item::Union(u) => {
            let all_public = field_records(writer, &syn::Fields::Named(u.fields.clone()));
            (TypeKind::Union, all_public && !non_exhaustive)
        }
        Item::Enum(e) => {
            for variant in &e.variants {
                let mut arguments = Vec::new();
                let mut refs = vec![owner.clone()];
                for (position, field) in variant.fields.iter().enumerate() {
                    let (ty, found) = writer.ty(module, &params, &field.ty);
                    merge(&mut refs, found);
                    arguments.push(Argument {
                        name: field
                            .ident
                            .as_ref()
                            .map_or_else(|| position.to_string(), ToString::to_string),
                        ty,
                    });
                }
                let named = matches!(variant.fields, syn::Fields::Named(_));
                let name = variant.ident.to_string();
                members.push(match (&variant.fields, named) {
                    (syn::Fields::Unit, _) => name.clone(),
                    (_, true) => format!(
                        "{name} {{ {} }}",
                        arguments
                            .iter()
                            .map(|a| format!("{}: {}", a.name, a.ty))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    (_, false) => format!(
                        "{name}({})",
                        arguments
                            .iter()
                            .map(|a| a.ty.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                });
                if hidden(&variant.attrs) {
                    continue;
                }
                variants.push(Variant::new(
                    format!("{id}.{name}"),
                    doc(&variant.attrs),
                    VariantContract {
                        fields: arguments,
                        named,
                        refs,
                    },
                ));
            }
            if non_exhaustive {
                members.clear();
            }
            (TypeKind::Enum, !non_exhaustive)
        }
        Item::Type(alias) => {
            let (written, refs) = writer.ty(module, &params, &alias.ty);
            target = Some(written);
            merge(&mut contract_refs, refs);
            (TypeKind::Alias, false)
        }
        _ => unreachable!("only types reach here"),
    };
    // Derived and hand-written alike, by trait; `Wire` is `wire_form`'s.
    let mut impls: BTreeSet<String> = derives.iter().filter(|d| *d != "Wire").cloned().collect();
    let mut impl_refs = Vec::new();
    for (impl_module, impl_params, trait_path) in trait_impls {
        // How a type crosses is `wire_form`'s.
        if trait_path
            .segments
            .last()
            .is_some_and(|s| s.ident == "Wire")
        {
            continue;
        }
        let (written, refs) = writer.path(*impl_module, impl_params, trait_path);
        impls.insert(written);
        merge(&mut impl_refs, refs);
    }
    let form = wire_forms::form_of(item_id, &attrs, &derives, forms);
    let wire_form = form.map(|form| {
        form.name(|ty, module| {
            let params = params.clone();
            writer.ty(module, &params, ty).0
        })
    });
    Pending {
        id,
        item: item_id,
        doc: doc(&attrs),
        paths: Vec::new(),
        impls: impls.into_iter().collect(),
        impl_refs,
        fields_hash: matches!(
            kind,
            TypeKind::Struct | TypeKind::TupleStruct | TypeKind::UnitStruct | TypeKind::Union
        )
        .then(|| {
            mod_surface::fields_hash_of(
                literal_fields
                    .iter()
                    .map(|(name, ty)| (name.as_str(), ty.as_str())),
            )
        }),
        contract: TypeContract {
            kind,
            generics,
            wire: wire_form.is_some(),
            wire_form,
            constructible,
            non_exhaustive,
            target,
            members,
            refs: contract_refs,
        },
        fields,
        variants,
    }
}

/// A capability key constant (`CapabilityKey<dyn T>`, `ItemCapabilityKey<…>`,
/// `EntityCapabilityKey<…, C>`), keyed by its `name`.
fn capability_key(
    writer: &mut Writer<'_, '_>,
    module: usize,
    path: &str,
    constant: &syn::ItemConst,
) -> Option<Capability> {
    let syn::Type::Path(ty) = &*constant.ty else {
        return None;
    };
    let last = ty.path.segments.last()?;
    let kind = match last.ident.to_string().as_str() {
        "CapabilityKey" => CapabilityKind::Block,
        "ItemCapabilityKey" => CapabilityKind::Item,
        "EntityCapabilityKey" => CapabilityKind::Entity,
        _ => return None,
    };
    let text = |expr: &syn::Expr| match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(key),
            ..
        }) => Some(key.value()),
        _ => None,
    };
    // A literal with a `name`, or a constructor given the name (`Key::new("horizon:x")`).
    let key = match &*constant.expr {
        syn::Expr::Struct(literal) => literal.fields.iter().find_map(|field| match &field.member {
            syn::Member::Named(name) if name == "name" => text(&field.expr),
            _ => None,
        }),
        syn::Expr::Call(call) => call.args.first().and_then(text),
        _ => None,
    }?;
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    let mut types = args.args.iter().filter_map(|arg| match arg {
        syn::GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    });
    let mut refs = Vec::new();
    let mut write = |writer: &mut Writer<'_, '_>, ty: Option<syn::Type>| {
        ty.map(|ty| {
            let (text, found) = writer.ty(module, &[], &ty);
            merge(&mut refs, found);
            text
        })
    };
    let handler = write(writer, types.next());
    let context = write(writer, types.next());
    Some(Capability::new(
        format!("capability:{key}"),
        doc(&constant.attrs),
        CapabilityContract {
            kind,
            constant: Some(path.to_owned()),
            handler,
            context,
            refs,
        },
    ))
}

/// The manifest's `capabilities` values: the variants of `horizon-mod-package`'s `Capability`.
fn manifest_capabilities(root: &Path, notes: &mut Vec<String>) -> Vec<Capability> {
    let path = root.join("crates/horizon-mod-package/src/manifest.rs");
    let parsed = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|source| syn::parse_file(&source).map_err(|e| e.to_string()));
    let file = match parsed {
        Ok(file) => file,
        Err(why) => {
            notes.push(format!("manifest capabilities: {why}"));
            return Vec::new();
        }
    };
    let Some(capability) = file.items.iter().find_map(|item| match item {
        Item::Enum(e) if e.ident == "Capability" => Some(e),
        _ => None,
    }) else {
        notes.push("manifest capabilities: no Capability enum in the manifest".into());
        return Vec::new();
    };
    // The key a manifest writes: serde's `rename` or `rename_all` on the enum.
    let serde_value = |attrs: &[syn::Attribute], name: &str| {
        let mut found = None;
        for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident(name) {
                    found = Some(meta.value()?.parse::<syn::LitStr>()?.value());
                } else if meta.input.peek(syn::Token![=]) {
                    let _: syn::Expr = meta.value()?.parse()?;
                }
                Ok(())
            });
        }
        found
    };
    let rename_all = serde_value(&capability.attrs, "rename_all");
    capability
        .variants
        .iter()
        .filter_map(|variant| {
            let ident = variant.ident.to_string();
            let key = match (serde_value(&variant.attrs, "rename"), rename_all.as_deref()) {
                (Some(key), _) => key,
                (None, Some("kebab-case")) => heck::ToKebabCase::to_kebab_case(ident.as_str()),
                (None, Some("snake_case")) => heck::ToSnakeCase::to_snake_case(ident.as_str()),
                (None, Some("lowercase")) => ident.to_lowercase(),
                (None, None) => ident.clone(),
                (None, Some(other)) => {
                    notes.push(format!(
                        "manifest capability {ident}: serde rename_all {other:?} is not read, not recorded"
                    ));
                    return None;
                }
            };
            Some(Capability::new(
                format!("capability:{key}"),
                doc(&variant.attrs),
                CapabilityContract {
                    kind: CapabilityKind::Manifest,
                    constant: None,
                    handler: None,
                    context: None,
                    refs: Vec::new(),
                },
            ))
        })
        .collect()
}

/// The re-exported procedural macros, from their definitions, and the crate's exported
/// `macro_rules!` macros.
fn macros(
    root: &Path,
    index: &mut Index<'_>,
    reexported: &[(Vec<String>, String)],
    notes: &mut Vec<String>,
) -> Result<Vec<Macro>, String> {
    let mut out = Vec::new();
    if !reexported.is_empty() {
        let path = root.join("crates/horizon-mod-api-macros/src/lib.rs");
        let source =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file = syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut defined: BTreeMap<String, (MacroKind, Vec<String>, String)> = BTreeMap::new();
        for item in &file.items {
            let Item::Fn(function) = item else { continue };
            for attr in &function.attrs {
                let entry = if attr.path().is_ident("proc_macro_attribute") {
                    (
                        function.sig.ident.to_string(),
                        MacroKind::Attribute,
                        Vec::new(),
                    )
                } else if attr.path().is_ident("proc_macro") {
                    (
                        function.sig.ident.to_string(),
                        MacroKind::Function,
                        Vec::new(),
                    )
                } else if attr.path().is_ident("proc_macro_derive") {
                    let Ok(args) = attr.parse_args_with(
                        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                    ) else {
                        continue;
                    };
                    let mut args = args.into_iter();
                    let Some(syn::Meta::Path(name)) = args.next() else {
                        continue;
                    };
                    let helpers = args
                        .filter_map(|meta| match meta {
                            syn::Meta::List(list) if list.path.is_ident("attributes") => list
                                .parse_args_with(
                                    syn::punctuated::Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated,
                                )
                                .ok(),
                            _ => None,
                        })
                        .flatten()
                        .map(|i| i.to_string())
                        .collect();
                    (print(name.to_token_stream()), MacroKind::Derive, helpers)
                } else {
                    continue;
                };
                defined.insert(entry.0, (entry.1, entry.2, doc(&function.attrs)));
            }
        }
        for (public, name) in reexported {
            match defined.get(name) {
                Some((kind, attributes, text)) => out.push(Macro::new(
                    format!("macro:{}", public.join("::")),
                    text.clone(),
                    MacroContract {
                        kind: *kind,
                        attributes: attributes.clone(),
                    },
                )),
                None => notes.push(format!(
                    "{}: re-exports {name}, which is no procedural macro of horizon-mod-api-macros",
                    public.join("::")
                )),
            }
        }
    }
    for krate in ["horizon_mod_api", "mod_api"] {
        let mut walker = walk::Walker::new(index);
        for module in walker.all_modules(krate)? {
            for item in &walker.index.modules[module].items {
                let Item::Macro(mac) = item else { continue };
                let Some(name) = &mac.ident else { continue };
                if !has_attr(&mac.attrs, "macro_export")
                    || hidden(&mac.attrs)
                    || !compiled(&mac.attrs)
                    || name.to_string().starts_with("__")
                {
                    continue;
                }
                out.push(Macro::new(
                    format!("macro:{name}"),
                    doc(&mac.attrs),
                    MacroContract {
                        kind: MacroKind::Declarative,
                        attributes: Vec::new(),
                    },
                ));
            }
        }
    }
    Ok(out)
}

fn hook_records(hooks: &[catalog::Hook], types: &[Type], notes: &mut Vec<String>) -> Vec<Hook> {
    let mut events: BTreeMap<&str, Vec<&Type>> = BTreeMap::new();
    for t in types {
        events.entry(leaf(&t.id)).or_default().push(t);
    }
    let mut out = Vec::new();
    for hook in hooks {
        let event = format!("{}Event", crate::hook_id(&hook.name));
        let fields = match events.get(event.as_str()).map(Vec::as_slice) {
            Some([event_type]) => event_type.fields.as_slice(),
            Some(several) => {
                notes.push(format!(
                    "hook {}: {} types are named {event}, so its arguments are not recorded",
                    hook.name,
                    several.len()
                ));
                &[]
            }
            None => {
                notes.push(format!(
                    "hook {}: its event {event} is not a mod_api type, so its arguments are not recorded",
                    hook.name
                ));
                &[]
            }
        };
        let output_name = hook
            .out
            .as_deref()
            .or((hook.contract == crate::Contract::Cancel).then_some("cancelled"));
        let mut arguments = Vec::new();
        let mut output = None;
        let mut refs = Vec::new();
        for field in fields {
            merge(&mut refs, field.contract.refs.clone());
            let argument = Argument {
                name: leaf(&field.id).to_owned(),
                ty: field.contract.ty.clone(),
            };
            if Some(argument.name.as_str()) == output_name {
                output = Some(argument);
            } else {
                arguments.push(argument);
            }
        }
        let snake = |kind: catalog::PartKind| match kind {
            catalog::PartKind::Block => "block",
            catalog::PartKind::BlockEntity => "block_entity",
            catalog::PartKind::Item => "item",
            catalog::PartKind::Menu => "menu",
            catalog::PartKind::Fluid => "fluid",
            catalog::PartKind::Entity => "entity",
        };
        out.push(Hook::new(
            format!("hook:{}", hook.name),
            hook.doc.clone(),
            HookEngine {
                target: hook.target.clone(),
            },
            Counterparts {
                neoforge: hook.counterparts.neoforge.clone(),
                fabric: hook.counterparts.fabric.clone(),
                paper: hook.counterparts.paper.clone(),
            },
            HookContract {
                context: hook.context.map(|context| {
                    match context {
                        catalog::PhaseContext::Registration => "registration",
                        catalog::PhaseContext::DataLoad => "data_load",
                        catalog::PhaseContext::Loop => "loop",
                        catalog::PhaseContext::Worldgen => "worldgen",
                    }
                    .to_owned()
                }),
                version: hook.version,
                side: match hook.side {
                    catalog::Side::Server => Side::Server,
                    catalog::Side::Client => Side::Client,
                    catalog::Side::Both => Side::Both,
                },
                contract: match hook.contract {
                    crate::Contract::Listen => HookKind::Listen,
                    crate::Contract::Cancel => HookKind::Cancel,
                    crate::Contract::Collect => HookKind::Collect,
                    crate::Contract::Replace => HookKind::Replace,
                },
                scope: hook.scope.as_ref().map(|scope| Scope {
                    kind: snake(scope.kind).to_owned(),
                    key: scope.key.clone(),
                }),
                cancellable: hook.cancellable,
                host: hook.host.map(|host| {
                    match host {
                        catalog::HostAccess::Read => "read",
                        catalog::HostAccess::Level => "level",
                    }
                    .to_owned()
                }),
                owner: hook.owner.clone(),
                answered_by_mod: hook.answered_by_mod,
                arguments,
                output,
                refs,
            },
        ));
    }
    out
}

/// One Part kind per kind the catalog's scoped hooks name, implemented through `<Kind>Part`.
fn part_records(hooks: &[Hook], traits: &[Trait], notes: &mut Vec<String>) -> Vec<Part> {
    let mut kinds: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for hook in hooks {
        if let Some(scope) = &hook.contract.scope {
            kinds
                .entry(scope.kind.clone())
                .or_default()
                .push(hook.id.clone());
        }
    }
    let mut out = Vec::new();
    for (kind, scoped) in kinds {
        let name = format!(
            "{}Part",
            heck::ToUpperCamelCase::to_upper_camel_case(kind.as_str())
        );
        let Some(found) = traits.iter().find(|t| leaf(&t.id) == name) else {
            notes.push(format!("part {kind}: no public trait {name}"));
            continue;
        };
        out.push(Part::new(
            format!("part:{kind}"),
            found.doc.clone(),
            scoped,
            PartContract {
                trait_id: found.id.clone(),
            },
        ));
    }
    out
}
