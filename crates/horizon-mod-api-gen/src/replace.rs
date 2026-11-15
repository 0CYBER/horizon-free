//! Function replacement: how a patched copy of a game function is installed in place of the
//! original, with the game prebuilt. The staging pass ([`stage`]) writes one check at the head of
//! every replaceable function of a staged source copy: a load of the function's slot in its
//! crate's `__REPLACE` array and, when a copy is installed there, a direct call of it. A function
//! that is an injection point shares that one check: its `#[point]` writes it, and while a mod
//! listens there the point's dispatcher is what the slot holds, with the copy behind it. Beside the
//! function it writes the slot's typed reader (`__fnr_read_slot_<index>`) and installer
//! (`__fnr_<name>`), which a mixin build's patch crate calls with its copy
//! ([`crate::patch`]). A function's check is part of its body, so every inlined copy of the
//! function carries it.
//!
//! Replaceable: every free function and inherent method of the game module that [`eligible`]
//! admits, and each function marked with `#[horizon_mod_api_macros::replaceable]` in the crates below it. The source tree
//! carries none of this: only a staged copy does, and everything the pass writes stays on the
//! line it annotates, so the staged source's line numbers are the pinned source's.
use crate::source::{self};
use mod_source::modules::ModuleFile;
use quote::quote;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use syn::{FnArg, ImplItem, Item, Pat, Type};

/// Whether a function with this signature and these attributes can take a replacement: its
/// slot holds one function pointer, so one compiled copy must stand for every call.
pub fn eligible(attrs: &[syn::Attribute], sig: &syn::Signature) -> bool {
    signature_exclusion(attrs, sig).is_none()
}

pub(crate) fn signature_exclusion(
    attrs: &[syn::Attribute],
    sig: &syn::Signature,
) -> Option<&'static str> {
    struct Opaque(bool);
    impl<'ast> syn::visit::Visit<'ast> for Opaque {
        fn visit_type_impl_trait(&mut self, _: &'ast syn::TypeImplTrait) {
            self.0 = true;
        }
    }
    let mut opaque = Opaque(false);
    syn::visit::Visit::visit_signature(&mut opaque, sig);
    let rules = [
        (opaque.0, "impl Trait"),
        (sig.constness.is_some(), "const"),
        (sig.asyncness.is_some(), "async"),
        (sig.unsafety.is_some(), "unsafe"),
        (sig.abi.is_some(), "extern ABI"),
        (sig.variadic.is_some(), "variadic"),
        (!lifetimes_only(&sig.generics), "function type/const generics"),
        (matches!(&sig.output, syn::ReturnType::Type(_, ty) if matches!(**ty, Type::Never(_))), "never return"),
        (sig.inputs.iter().any(|input| matches!(input, FnArg::Receiver(r) if r.reference.is_none() || r.colon_token.is_some())), "by-value/typed receiver"),
        (sig.inputs.iter().any(|input| match input { FnArg::Receiver(r) => !r.attrs.is_empty(), FnArg::Typed(t) => !t.attrs.is_empty() }), "parameter attributes"),
    ];
    rules
        .into_iter()
        .find_map(|(applies, name)| applies.then_some(name))
        .or_else(|| {
            attrs.iter().find_map(|attr| {
                let path = attr.path();
                if path.is_ident("cfg") || path.is_ident("cfg_attr") {
                    Some("conditional function")
                } else if path.is_ident("test") {
                    Some("test function")
                } else if path.is_ident("track_caller") {
                    Some("track_caller")
                } else if path.is_ident("inline")
                    && attr
                        .parse_args::<syn::Ident>()
                        .is_ok_and(|arg| arg == "always")
                {
                    Some("inline(always)")
                } else {
                    None
                }
            })
        })
}

/// Whether `generics` declares lifetimes only: a type or const parameter means one function per
/// instance, which one slot can't replace.
pub fn lifetimes_only(generics: &syn::Generics) -> bool {
    generics
        .params
        .iter()
        .all(|param| matches!(param, syn::GenericParam::Lifetime(_)))
}

/// The installer of the function `name`: `__fnr_<name>`, in a free function's module or on an
/// inherent method's type; `__fnr_as_<Trait>_<name>` on the type for a method of its
/// `impl Trait`.
pub fn installer(trait_name: Option<&syn::Ident>, name: &syn::Ident) -> syn::Ident {
    match trait_name {
        Some(trait_name) => {
            quote::format_ident!("__fnr_as_{trait_name}_{}", source::unraw(name))
        }
        None => quote::format_ident!("__fnr_{}", source::unraw(name)),
    }
}

/// The trait of `block` when its methods can be replaced: a trait one of the game's own crates
/// declares (`traits`, by name), written without generic arguments, implemented safely for a
/// type its own crate declares: the installer is an inherent function of the type. The type is
/// written as one name, which exactly one module of the crate declares (`types`), in the impl's
/// file or any other, and the file does not import from another crate (`foreign`). Another
/// crate's trait (`Ord`, `Iterator`, `Drop`) is the caller's contract with that crate, and often
/// a hot path.
pub fn replaceable_trait<'a>(
    module: &str,
    block: &'a syn::ItemImpl,
    traits: &BTreeSet<String>,
    types: &OwnerTypes,
    foreign: &BTreeSet<String>,
) -> Option<&'a syn::Ident> {
    let (_, path, _) = block.trait_.as_ref()?;
    trait_exclusion(module, block, traits, types, foreign)
        .is_none()
        .then(|| &path.segments.last().expect("a trait path has a name").ident)
}

fn trait_exclusion(
    module: &str,
    block: &syn::ItemImpl,
    traits: &BTreeSet<String>,
    types: &OwnerTypes,
    foreign: &BTreeSet<String>,
) -> Option<&'static str> {
    let (negative, path, _) = block.trait_.as_ref()?;
    let last = path.segments.last().expect("a trait path has a name");
    let Type::Path(owner) = &*block.self_ty else {
        return Some("non-path trait owner");
    };
    let [owner] = owner.path.segments.iter().collect::<Vec<_>>()[..] else {
        return Some("path-written trait owner");
    };
    let owner = owner.ident.to_string();
    let undeclared = types.exclusion(module, &owner);
    [
        (negative.is_some(), "negative trait impl"),
        (block.unsafety.is_some(), "unsafe trait impl"),
        (!last.arguments.is_none(), "trait generic arguments"),
        (!traits.contains(&last.ident.to_string()), "foreign trait"),
        (undeclared.is_some(), undeclared.unwrap_or_default()),
        (foreign.contains(&owner), "imported trait owner"),
    ]
    .into_iter()
    .find_map(|(applies, name)| applies.then_some(name))
}

/// Why a mixin's target in the trait implementation `block` has no installer, for the refusal;
/// `None` when it has one.
pub(crate) fn trait_refusal(
    module: &str,
    block: &syn::ItemImpl,
    traits: &BTreeSet<String>,
    types: &OwnerTypes,
    foreign: &BTreeSet<String>,
) -> Option<String> {
    let reason = trait_exclusion(module, block, traits, types, foreign)?;
    Some(if reason == AMBIGUOUS_OWNER {
        format!(
            "its trait implementation has no replacement installer: owner type name `{}` is ambiguous in the crate: more than one module declares it, and the `impl`'s module neither declares it nor imports it from one of them",
            quote::ToTokens::to_token_stream(&block.self_ty)
        )
    } else {
        format!("its trait implementation has no replacement installer ({reason})")
    })
}

const AMBIGUOUS_OWNER: &str = "ambiguous trait owner";

/// What a crate's source says about the names of its types: each type with the modules that
/// declare it, and each module's `use`s, gathered over every file of the module. The staging pass
/// and the patch generator fill it from the same files, so both give a trait implementation's
/// owner the same answer, in whatever file the `impl` is.
#[derive(Default)]
pub struct OwnerTypes {
    types: BTreeMap<String, BTreeSet<String>>,
    modules: BTreeMap<String, Uses>,
}

/// One module's `use`s.
#[derive(Default)]
struct Uses {
    /// Each name a `use` brings in, under the name it has here (a renamed import under its new
    /// one), with where the path takes it from.
    named: BTreeMap<String, Import>,
    globs: Vec<Glob>,
}

/// Where a `use` takes a name from: `original` is the name it has there.
enum Import {
    /// A module of the crate, written from `crate`, `self` or `super`.
    Module { from: String, original: String },
    /// A path written from a bare name, as [`Glob::Bare`].
    Bare { path: Vec<String>, original: String },
}

/// A `use …::*`.
enum Glob {
    /// Of a module of the crate, written from `crate`, `self` or `super`.
    Module(String),
    /// Written from a bare name: a child module, an enum's variants, the crate under its own
    /// name, or another crate. Which one is known once every module is read.
    Bare(Vec<String>),
}

/// What a name is in a module, as far as its `use`s say.
enum Binding {
    /// The type the module `0` declares.
    Local(String),
    /// Another crate's.
    Foreign,
    /// A `use` names it and does not lead to a type this pass reads, or a glob of another crate
    /// may name it.
    Unresolved,
    /// Two different things by two globs.
    Ambiguous,
    /// No `use` names it.
    NotFound,
}

impl OwnerTypes {
    /// Adds what `items`, a file of `module`, declare and import, inline modules included.
    pub fn declare(&mut self, module: &str, items: &[Item]) {
        fn parent(module: &str) -> String {
            module
                .rsplit_once("::")
                .map_or(module, |(parent, _)| parent)
                .to_owned()
        }
        fn bare(tree: &syn::UseTree, path: &mut Vec<String>, uses: &mut Uses) {
            let import = |path: &[String], original: &syn::Ident| Import::Bare {
                path: path.to_vec(),
                original: original.to_string(),
            };
            match tree {
                syn::UseTree::Path(next) => {
                    path.push(next.ident.to_string());
                    bare(&next.tree, path, uses);
                    path.pop();
                }
                syn::UseTree::Name(name) if name.ident == "self" => {}
                syn::UseTree::Name(name) => {
                    uses.named
                        .insert(name.ident.to_string(), import(path.as_slice(), &name.ident));
                }
                syn::UseTree::Rename(rename) => {
                    uses.named.insert(
                        rename.rename.to_string(),
                        import(path.as_slice(), &rename.ident),
                    );
                }
                syn::UseTree::Glob(_) => uses.globs.push(Glob::Bare(path.clone())),
                syn::UseTree::Group(group) => {
                    for tree in &group.items {
                        bare(tree, path, uses);
                    }
                }
            }
        }
        fn local(tree: &syn::UseTree, at: &str, uses: &mut Uses) {
            let import = |original: &syn::Ident| Import::Module {
                from: at.to_owned(),
                original: original.to_string(),
            };
            match tree {
                syn::UseTree::Path(path) if path.ident == "super" => {
                    local(&path.tree, &parent(at), uses)
                }
                syn::UseTree::Path(path) => {
                    local(&path.tree, &format!("{at}::{}", path.ident), uses)
                }
                syn::UseTree::Name(name) if name.ident == "self" => {}
                syn::UseTree::Name(name) => {
                    uses.named
                        .insert(name.ident.to_string(), import(&name.ident));
                }
                // A module under another name names no type.
                syn::UseTree::Rename(rename) if rename.ident == "self" => {}
                syn::UseTree::Rename(rename) => {
                    uses.named
                        .insert(rename.rename.to_string(), import(&rename.ident));
                }
                syn::UseTree::Glob(_) => uses.globs.push(Glob::Module(at.to_owned())),
                syn::UseTree::Group(group) => {
                    for tree in &group.items {
                        local(tree, at, uses);
                    }
                }
            }
        }
        let mut uses = self.modules.remove(module).unwrap_or_default();
        for item in items {
            let name = match item {
                Item::Struct(item) => &item.ident,
                Item::Enum(item) => &item.ident,
                Item::Union(item) => &item.ident,
                Item::Mod(item) => {
                    if let Some((_, inner)) = &item.content {
                        self.declare(&format!("{module}::{}", item.ident), inner);
                    }
                    continue;
                }
                Item::Use(item) => {
                    let from = match &item.tree {
                        syn::UseTree::Path(path) if path.ident == "crate" => {
                            Some(module.split("::").next().unwrap_or(module).to_owned())
                        }
                        syn::UseTree::Path(path) if path.ident == "self" => Some(module.to_owned()),
                        syn::UseTree::Path(path) if path.ident == "super" => Some(parent(module)),
                        _ => None,
                    };
                    match (&item.tree, from) {
                        (syn::UseTree::Path(path), Some(from)) => {
                            local(&path.tree, &from, &mut uses)
                        }
                        (tree, _) => bare(tree, &mut Vec::new(), &mut uses),
                    }
                    continue;
                }
                _ => continue,
            };
            self.types
                .entry(name.to_string())
                .or_default()
                .insert(module.to_owned());
        }
        self.modules.insert(module.to_owned(), uses);
    }

    /// The module a path written from a bare name leads to: `Ok(None)` for one that holds no
    /// type (an enum's variants), `Err(())` for another crate's.
    fn bare_module(&self, module: &str, path: &[String]) -> Result<Option<String>, ()> {
        let Some(first) = path.first() else {
            return Err(());
        };
        if self.modules.contains_key(&format!("{module}::{first}")) {
            Ok(Some(format!("{module}::{}", path.join("::"))))
        } else if self
            .types
            .get(first)
            .is_some_and(|modules| modules.contains(module))
        {
            Ok(None)
        } else if module.split("::").next() == Some(first.as_str()) {
            // The crate under its own name.
            Ok(Some(path.join("::")))
        } else {
            Err(())
        }
    }

    /// What `name` is in `module`: the module's own type; else what a `use` names, followed to
    /// where it leads under the name it has there; else what its globs bring in, where two
    /// different answers are no answer.
    fn binding(&self, module: &str, name: &str, seen: &mut BTreeSet<(String, String)>) -> Binding {
        if !seen.insert((module.to_owned(), name.to_owned())) {
            return Binding::NotFound;
        }
        if self
            .types
            .get(name)
            .is_some_and(|modules| modules.contains(module))
        {
            return Binding::Local(module.to_owned());
        }
        let Some(uses) = self.modules.get(module) else {
            return Binding::NotFound;
        };
        if let Some(import) = uses.named.get(name) {
            let (from, original) = match import {
                Import::Module { from, original } => (from.clone(), original),
                Import::Bare { path, original } => match self.bare_module(module, path) {
                    Ok(Some(from)) => (from, original),
                    Ok(None) | Err(()) => return Binding::Foreign,
                },
            };
            // The `use` binds the name: where it leads nowhere this pass reads, nothing else
            // may stand in for it.
            return match self.binding(&from, original, seen) {
                Binding::NotFound => Binding::Unresolved,
                found => found,
            };
        }
        // One module reached through two globs is one answer: `None` here is another crate's.
        let mut found: BTreeSet<Option<String>> = BTreeSet::new();
        let mut unresolved = false;
        for glob in &uses.globs {
            let from = match glob {
                Glob::Module(from) => from.clone(),
                Glob::Bare(path) => match self.bare_module(module, path) {
                    Ok(Some(from)) => from,
                    Ok(None) => continue,
                    Err(()) => {
                        unresolved = true;
                        continue;
                    }
                },
            };
            match self.binding(&from, name, seen) {
                Binding::Local(declaring) => drop(found.insert(Some(declaring))),
                Binding::Foreign => drop(found.insert(None)),
                Binding::Unresolved => unresolved = true,
                Binding::Ambiguous => return Binding::Ambiguous,
                Binding::NotFound => {}
            }
        }
        let mut found = found.into_iter();
        match (found.next(), found.next()) {
            (Some(Some(declaring)), None) => Binding::Local(declaring),
            (Some(None), None) => Binding::Foreign,
            (Some(_), Some(_)) => Binding::Ambiguous,
            (None, _) if unresolved => Binding::Unresolved,
            (None, _) => Binding::NotFound,
        }
    }

    /// Why `owner`, named by an `impl` in `module`, can't carry an installer. The name must be a
    /// type of the crate: the one the module declares or its `use`s lead to, a renamed import
    /// followed to the type it renames; or, where no `use` names it at all, the crate's only
    /// type of that name. A name that a `use` binds to another crate's type, or to something
    /// this pass can't follow, is refused, as is one that several modules declare with nothing
    /// to choose between them.
    fn exclusion(&self, module: &str, owner: &str) -> Option<&'static str> {
        match self.binding(module, owner, &mut BTreeSet::new()) {
            Binding::Local(_) => None,
            Binding::Foreign | Binding::Unresolved => Some("imported trait owner"),
            Binding::Ambiguous => Some(AMBIGUOUS_OWNER),
            Binding::NotFound => match self.types.get(owner).map_or(0, BTreeSet::len) {
                0 => Some("foreign trait owner"),
                1 => None,
                _ => Some(AMBIGUOUS_OWNER),
            },
        }
    }
}

/// The names `items`' `use`s bring in from other crates: paths that don't start at `crate`,
/// `self` or `super`.
pub fn foreign_imports(items: &[Item]) -> BTreeSet<String> {
    fn leaves(tree: &syn::UseTree, out: &mut BTreeSet<String>) {
        match tree {
            syn::UseTree::Path(path) => leaves(&path.tree, out),
            syn::UseTree::Name(name) => drop(out.insert(name.ident.to_string())),
            syn::UseTree::Rename(rename) => drop(out.insert(rename.rename.to_string())),
            syn::UseTree::Group(group) => group.items.iter().for_each(|tree| leaves(tree, out)),
            syn::UseTree::Glob(_) => {}
        }
    }
    let mut out = BTreeSet::new();
    for item in items {
        if let Item::Use(item) = item
            && !matches!(&item.tree, syn::UseTree::Path(path) if path.ident == "crate" || path.ident == "self" || path.ident == "super")
        {
            leaves(&item.tree, &mut out);
        }
    }
    out
}

/// `sig` with each type it names through `Self::` (an associated type of the trait) written
/// `<Self as Trait>::`: the method's pointer type is written outside the trait's `impl`, where
/// the short form doesn't resolve.
pub fn qualified(sig: &syn::Signature, trait_path: &syn::Path) -> syn::Signature {
    struct Qualify<'a>(&'a syn::Path);
    impl syn::visit_mut::VisitMut for Qualify<'_> {
        fn visit_type_path_mut(&mut self, ty: &mut syn::TypePath) {
            if ty.qself.is_none()
                && ty.path.segments.len() > 1
                && ty.path.segments[0].ident == "Self"
            {
                let rest = ty.path.segments.iter().skip(1);
                let trait_path = self.0;
                *ty = syn::parse_quote!(<Self as #trait_path>::#(#rest)::*);
            }
            syn::visit_mut::visit_type_path_mut(self, ty);
        }
    }
    let mut sig = sig.clone();
    syn::visit_mut::VisitMut::visit_signature_mut(&mut Qualify(trait_path), &mut sig);
    sig
}

/// Whether `sig` still names something through `Self::` (an associated constant of the trait, in
/// an array length): that has no form outside the trait's `impl`.
fn names_associated(sig: &syn::Signature) -> bool {
    struct Find(bool);
    impl<'ast> syn::visit::Visit<'ast> for Find {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            self.0 |= path.segments.len() > 1 && path.segments[0].ident == "Self";
            syn::visit::visit_path(self, path);
        }
    }
    let mut find = Find(false);
    syn::visit::Visit::visit_signature(&mut find, sig);
    find.0
}

/// The names of the traits the game's own crates declare: every `trait` item under `crates/`.
pub fn declared_traits(root: &Path) -> Result<BTreeSet<String>, String> {
    fn walk(dir: &Path, out: &mut BTreeSet<String>) -> Result<(), String> {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("{}: {error}", dir.display())),
        };
        for entry in entries {
            let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let text =
                    fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                for (at, _) in text.match_indices("trait ") {
                    // An item keyword, not the end of a longer word or a doc sentence's.
                    let before = text[..at].lines().next_back().unwrap_or_default().trim();
                    if !matches!(
                        before,
                        "" | "pub" | "unsafe" | "pub unsafe" | "pub(crate)" | "pub(super)"
                    ) {
                        continue;
                    }
                    let name: String = text[at + "trait ".len()..]
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        out.insert(name);
                    }
                }
            }
        }
        Ok(())
    }
    let mut traits = BTreeSet::new();
    let crates = root.join("crates");
    for entry in fs::read_dir(&crates).map_err(|e| format!("{}: {e}", crates.display()))? {
        let package = entry.map_err(|e| e.to_string())?.path();
        walk(&package.join("src"), &mut traits)?;
    }
    Ok(traits)
}

/// Elided lifetimes of a method's return borrow from its receiver: named.
struct ElideTo(syn::Lifetime);

impl syn::visit_mut::VisitMut for ElideTo {
    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        if reference.lifetime.is_none() {
            reference.lifetime = Some(self.0.clone());
        }
        syn::visit_mut::visit_type_reference_mut(self, reference);
    }
    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
        if lifetime.ident == "_" {
            *lifetime = self.0.clone();
        }
    }
    // A function pointer or trait object in the return elides on its own.
    fn visit_type_bare_fn_mut(&mut self, _: &mut syn::TypeBareFn) {}
    fn visit_type_trait_object_mut(&mut self, _: &mut syn::TypeTraitObject) {}
}

/// The function-pointer type of `sig`, as written inside its `impl`: the receiver as the first
/// parameter (`&Self` or `&mut Self`), the function's own lifetimes bound for any call, and a
/// return that borrows from the receiver as the method's elision says.
pub fn pointer_type(sig: &syn::Signature) -> proc_macro2::TokenStream {
    // The function's own lifetimes are renamed: a `for<>` binder may not shadow them where
    // they are in scope.
    struct Rename(Vec<(syn::Ident, syn::Lifetime)>);
    impl syn::visit_mut::VisitMut for Rename {
        fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
            if let Some((_, fresh)) = self.0.iter().find(|(name, _)| *name == lifetime.ident) {
                *lifetime = fresh.clone();
            }
        }
    }
    let mut rename = Rename(
        sig.generics
            .lifetimes()
            .enumerate()
            .map(|(index, param)| {
                (
                    param.lifetime.ident.clone(),
                    syn::Lifetime::new(&format!("'__fnr_l{index}"), proc_macro2::Span::call_site()),
                )
            })
            .collect(),
    );
    let mut lifetimes: Vec<syn::Lifetime> =
        rename.0.iter().map(|(_, fresh)| fresh.clone()).collect();
    let mut output = sig.output.clone();
    syn::visit_mut::VisitMut::visit_return_type_mut(&mut rename, &mut output);
    let first = sig.receiver().map(|written| {
        let named = written.reference.as_ref().and_then(|(_, lifetime)| {
            lifetime.clone().map(|mut lifetime| {
                syn::visit_mut::VisitMut::visit_lifetime_mut(&mut rename, &mut lifetime);
                lifetime
            })
        });
        let lifetime = named.unwrap_or_else(|| {
            let fresh = syn::Lifetime::new("'__fnr_s", proc_macro2::Span::call_site());
            lifetimes.push(fresh.clone());
            fresh
        });
        syn::visit_mut::VisitMut::visit_return_type_mut(
            &mut ElideTo(lifetime.clone()),
            &mut output,
        );
        let mutability = &written.mutability;
        quote!(&#lifetime #mutability Self,)
    });
    let types = sig.inputs.iter().filter_map(|input| match input {
        FnArg::Typed(typed) => {
            let mut ty = (*typed.ty).clone();
            syn::visit_mut::VisitMut::visit_type_mut(&mut rename, &mut ty);
            Some(ty)
        }
        FnArg::Receiver(_) => None,
    });
    let bound = if lifetimes.is_empty() {
        quote!()
    } else {
        quote!(for<#(#lifetimes),*>)
    };
    quote!(#bound fn(#first #(#types),*) #output)
}

/// A replaceable function, as found in a parsed file.
struct Found<'a> {
    module: String,
    attrs: &'a [syn::Attribute],
    sig: &'a syn::Signature,
    body: &'a syn::Block,
    /// The `impl` it is a method of.
    owner: Option<&'a syn::ItemImpl>,
    /// That `impl`'s trait, when it is one's.
    trait_name: Option<&'a syn::Ident>,
    /// Where the item that holds it ends: the `impl` block, or the function.
    end: proc_macro2::LineColumn,
}

impl Found<'_> {
    /// The `#[point(…)]` the point staging pass put on it, when it is an injection point.
    fn point(&self) -> Option<&syn::MetaList> {
        self.attrs.iter().find_map(|attr| match &attr.meta {
            syn::Meta::List(list)
                if list
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "point") =>
            {
                Some(list)
            }
            _ => None,
        })
    }

    /// Its path, as a mixin's `target` names it.
    fn target(&self) -> String {
        match (self.owner, self.trait_name) {
            (Some(block), Some(trait_name)) => format!(
                "{}::<{} as {trait_name}>::{}",
                self.module,
                source::owner(&block.self_ty),
                self.sig.ident
            ),
            (Some(block), None) => format!(
                "{}::{}::{}",
                self.module,
                source::owner(&block.self_ty),
                self.sig.ident
            ),
            (None, _) => format!("{}::{}", self.module, self.sig.ident),
        }
    }
}

/// The replaceable functions among `items`: free functions, inherent methods and methods of the
/// game's own traits (`traits`) that every build compiles, inline modules included.
fn functions<'a>(
    module: &str,
    items: &'a [Item],
    traits: &BTreeSet<String>,
    types: &OwnerTypes,
    out: &mut Vec<Found<'a>>,
) {
    let foreign = foreign_imports(items);
    for item in items {
        match item {
            Item::Mod(inline) if !mod_source::modules::gated(&inline.attrs) => {
                if let Some((_, inner)) = &inline.content
                    && !mod_source::modules::test_only(&inline.attrs)
                {
                    functions(
                        &format!("{module}::{}", inline.ident),
                        inner,
                        traits,
                        types,
                        out,
                    );
                }
            }
            Item::Fn(function)
                if !mod_source::modules::gated(&function.attrs)
                    && eligible(&function.attrs, &function.sig) =>
            {
                out.push(Found {
                    module: module.to_owned(),
                    attrs: &function.attrs,
                    sig: &function.sig,
                    body: &function.block,
                    owner: None,
                    trait_name: None,
                    end: function.block.brace_token.span.close().end(),
                });
            }
            Item::Impl(block)
                if !mod_source::modules::gated(&block.attrs)
                    && lifetimes_only(&block.generics)
                    && matches!(&*block.self_ty, Type::Path(ty) if ty.qself.is_none()) =>
            {
                let trait_name = replaceable_trait(module, block, traits, types, &foreign);
                if block.trait_.is_some() && trait_name.is_none() {
                    continue;
                }
                for member in &block.items {
                    if let ImplItem::Fn(method) = member
                        && eligible(&method.attrs, &method.sig)
                        && !block.trait_.as_ref().is_some_and(|(_, path, _)| {
                            names_associated(&qualified(&method.sig, path))
                        })
                    {
                        out.push(Found {
                            module: module.to_owned(),
                            attrs: &method.attrs,
                            sig: &method.sig,
                            body: &method.block,
                            owner: Some(block),
                            trait_name,
                            end: block.brace_token.span.close().end(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// Module-scope function definitions, partitioned by their first exclusion rule. Local functions
/// inside bodies and macro expansions are not source-level replacement candidates.
fn inventory(
    module: &str,
    items: &[Item],
    traits: &BTreeSet<String>,
    types: &OwnerTypes,
    inherited: Option<&'static str>,
    counts: &mut BTreeMap<&'static str, usize>,
) {
    fn record(
        attrs: &[syn::Attribute],
        sig: &syn::Signature,
        inherited: Option<&'static str>,
        counts: &mut BTreeMap<&'static str, usize>,
    ) {
        let reason = inherited
            .or_else(|| signature_exclusion(attrs, sig))
            .unwrap_or("eligible");
        *counts.entry(reason).or_default() += 1;
    }
    let foreign = foreign_imports(items);
    for item in items {
        match item {
            Item::Mod(inline) => {
                if let Some((_, inner)) = &inline.content {
                    let reason = inherited.or_else(|| {
                        (mod_source::modules::gated(&inline.attrs)
                            || mod_source::modules::test_only(&inline.attrs))
                        .then_some("conditional/test module")
                    });
                    inventory(
                        &format!("{module}::{}", inline.ident),
                        inner,
                        traits,
                        types,
                        reason,
                        counts,
                    );
                }
            }
            Item::Fn(function) => record(&function.attrs, &function.sig, inherited, counts),
            Item::Impl(block) => {
                let reason = inherited.or_else(|| {
                    if mod_source::modules::gated(&block.attrs) {
                        Some("conditional impl")
                    } else if !lifetimes_only(&block.generics) {
                        Some("impl type/const generics")
                    } else if !matches!(&*block.self_ty, Type::Path(ty) if ty.qself.is_none()) {
                        Some("non-path/qualified impl owner")
                    } else {
                        trait_exclusion(module, block, traits, types, &foreign)
                    }
                });
                for member in &block.items {
                    if let ImplItem::Fn(method) = member {
                        let reason = reason.or_else(|| {
                            block
                                .trait_
                                .as_ref()
                                .filter(|(_, path, _)| {
                                    names_associated(&qualified(&method.sig, path))
                                })
                                .map(|_| "associated constant signature")
                        });
                        record(&method.attrs, &method.sig, reason, counts);
                    }
                }
            }
            Item::Trait(item) => {
                for member in &item.items {
                    if let syn::TraitItem::Fn(method) = member
                        && method.default.is_some()
                    {
                        record(
                            &method.attrs,
                            &method.sig,
                            Some("trait default body"),
                            counts,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// Marked lower-crate functions and their source locations.
pub(crate) fn marked(root: &Path) -> Result<Vec<(String, String)>, String> {
    fn scan(module: &str, items: &[Item], out: &mut Vec<(String, String)>) -> Result<(), String> {
        for item in items {
            match item {
                Item::Fn(f) if source::marker(&f.attrs, "replaceable").is_some() => {
                    let path = format!("{module}::{}", f.sig.ident);
                    if let Some(reason) = signature_exclusion(&f.attrs, &f.sig) {
                        return Err(format!(
                            "#[horizon_mod_api_macros::replaceable] on {path}: {reason}"
                        ));
                    }
                    out.push((
                        format!("#[horizon_mod_api_macros::replaceable] on {path}"),
                        path,
                    ));
                }
                Item::Impl(block) => {
                    for member in &block.items {
                        if let ImplItem::Fn(f) = member
                            && source::marker(&f.attrs, "replaceable").is_some()
                        {
                            let path = source::plain_path(&format!(
                                "{module}::{}::{}",
                                source::owner(&block.self_ty),
                                f.sig.ident
                            ));
                            let reason = signature_exclusion(&f.attrs, &f.sig)
                                .or_else(|| {
                                    (!lifetimes_only(&block.generics))
                                        .then_some("impl type/const generics")
                                })
                                .or_else(|| {
                                    mod_source::modules::gated(&block.attrs)
                                        .then_some("conditional impl")
                                });
                            if let Some(reason) = reason {
                                return Err(format!(
                                    "#[horizon_mod_api_macros::replaceable] on {path}: {reason}"
                                ));
                            }
                            out.push((
                                format!("#[horizon_mod_api_macros::replaceable] on {path}"),
                                path,
                            ));
                        }
                    }
                }
                Item::Mod(m) => {
                    if let Some((_, items)) = &m.content {
                        scan(&format!("{module}::{}", m.ident), items, out)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    let crates = root.join("crates");
    if !crates.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(&crates).map_err(|e| e.to_string())? {
        let package = entry.map_err(|e| e.to_string())?.path();
        if !package.join("Cargo.toml").exists() {
            continue;
        }
        let name = package
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('-', "_");
        if name == "horizon_client" {
            continue;
        }
        let manifest: toml::Value = toml::from_str(
            &fs::read_to_string(package.join("Cargo.toml")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let lib = package.join(
            manifest
                .get("lib")
                .and_then(|v| v.get("path"))
                .and_then(toml::Value::as_str)
                .unwrap_or("src/lib.rs"),
        );
        if !lib.exists() {
            continue;
        }
        for file in source::module_files(
            name.clone(),
            lib.clone(),
            lib.parent().unwrap().to_owned(),
            true,
        )? {
            let text = fs::read_to_string(&file.path).map_err(|e| e.to_string())?;
            if !text.contains("replaceable") {
                continue;
            }
            let parsed =
                syn::parse_file(&text).map_err(|e| format!("{}: {e}", file.path.display()))?;
            scan(&file.module, &parsed.items, &mut out)?;
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The refusal of a target below the game crate without the marker.
pub const UNMARKED: &str = "a function below the game crate takes a replacement only when it carries #[horizon_mod_api_macros::replaceable]; ask for it in an issue or pull request on the game";

/// What [`stage`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Staged {
    /// Functions given a replacement slot.
    pub functions: usize,
    /// Files rewritten.
    pub files: usize,
    /// Crates given a slot array.
    pub crates: usize,
    /// The functions given a slot, by their key in the function index.
    pub slots: BTreeSet<String>,
}

/// The library name of the package at `package`: its `[lib]` name, or its own with underscores.
pub(crate) fn library_name(package: &Path) -> Result<String, String> {
    let manifest = package.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let cargo: toml::Table =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let name = |table: &str| {
        cargo
            .get(table)
            .and_then(|table| table.get("name"))
            .and_then(toml::Value::as_str)
    };
    name("lib")
        .or(name("package"))
        .map(|name| name.replace('-', "_"))
        .ok_or_else(|| format!("{}: no package name", manifest.display()))
}

/// One crate's files in the staged copy.
struct Crate {
    /// Its library's name, as the game crate names it.
    name: String,
    /// Its library root, where its slot array goes.
    lib: PathBuf,
    files: Vec<ModuleFile>,
    /// Whether only its marked functions are replaceable.
    marked_only: bool,
    /// Its package folder.
    package: PathBuf,
}

/// The staging pass: in the staged copy at `root`, in place, gives every replaceable function its
/// head check, reader and installer, and each crate that has one its slot array. Run on a copy,
/// never on a working tree.
pub fn stage(root: &Path) -> Result<Staged, String> {
    let traits = declared_traits(root)?;
    let list = marked(root)?;
    let mut used = vec![false; list.len()];
    let client = root.join("crates/horizon-client/src");
    let game = root.join("crates/horizon-client");
    let mut crates = vec![Crate {
        name: "horizon_client".into(),
        lib: client.join("lib.rs"),
        files: source::game_files(&client, true)?,
        marked_only: false,
        package: game.clone(),
    }];
    let lower: BTreeSet<&str> = list
        .iter()
        .map(|(_, path)| path.split("::").next().unwrap_or_default())
        .collect();
    for name in lower {
        if name == "horizon_client" {
            return Err(format!(
                "{name}: every function of the game crate's game module is replaceable already"
            ));
        }
        let (package, lib) = source::crate_root(root, name)?;
        // The game crate arms every marked crate: one built on the game crate would be a cycle.
        if built_on(&package, &game)? {
            return Err(format!(
                "{name}: it is built on horizon-client, so the game crate can't arm its replaceable functions; a crate with #[horizon_mod_api_macros::replaceable] must sit below the game crate"
            ));
        }
        crates.push(Crate {
            name: library_name(&package)?,
            lib,
            files: source::crate_files(root, name)?,
            marked_only: true,
            package,
        });
    }
    let mut staged = Staged::default();
    for krate in &crates {
        let mut count = 0;
        let mut cells = 0;
        let mut armers = Vec::new();
        let mut trait_methods = 0;
        let mut counts = BTreeMap::new();
        // Each file's text as the pass leaves it, the crate root's included.
        let mut texts: BTreeMap<&Path, String> = BTreeMap::new();
        let mut parsed = Vec::new();
        let mut types = OwnerTypes::default();
        for file in &krate.files {
            let text = fs::read_to_string(&file.path)
                .map_err(|e| format!("{}: {e}", file.path.display()))?;
            let items = syn::parse_file(&text)
                .map_err(|e| format!("{}: {e}", file.path.display()))?
                .items;
            types.declare(&file.module, &items);
            parsed.push((file, text, items));
        }
        for (file, text, items) in &parsed {
            if !krate.marked_only {
                inventory(&file.module, items, &traits, &types, None, &mut counts);
            }
            let mut found = Vec::new();
            functions(&file.module, items, &traits, &types, &mut found);
            if krate.marked_only {
                found.retain(|found| {
                    let target = source::plain_path(&found.target());
                    list.iter()
                        .position(|(_, path)| *path == target)
                        .inspect(|&index| used[index] = true)
                        .is_some()
                });
            }
            staged.slots.extend(
                found
                    .iter()
                    .map(|function| crate::functions::key(&function.target())),
            );
            trait_methods += found
                .iter()
                .filter(|function| function.trait_name.is_some())
                .count();
            count += found.len();
            let Some(text) = rewrite(&file.path, text, &found, &mut cells, &mut armers)? else {
                continue;
            };
            staged.files += 1;
            texts.insert(&file.path, text);
        }
        eprintln!(
            "replacement slots: {}: total={count}, trait_methods={trait_methods}",
            krate.lib.display()
        );
        if !krate.marked_only {
            debug_assert_eq!(counts.get("eligible").copied().unwrap_or_default(), count);
            eprintln!(
                "replacement exclusions (first rule, module-scope definitions): {}",
                serde_json::to_string(&counts).expect("static string counts serialize")
            );
        }
        if count == 0 {
            continue;
        }
        let root_text = match texts.remove(krate.lib.as_path()) {
            Some(text) => text,
            None => fs::read_to_string(&krate.lib)
                .map_err(|e| format!("{}: {e}", krate.lib.display()))?,
        };
        // The generated readers cast a slot's pointer back to its function's type: the staged
        // crate allows that cast by name where the source forbids unsafe code outright.
        let mut root_text =
            root_text.replacen("#![forbid(unsafe_code)]", "#![deny(unsafe_code)]", 1);
        root_text.push_str(&format!(
            "\n/// The replacement of each replaceable function of this crate, or null: see `horizon_mod_api_gen::replace`.\n#[doc(hidden)]\npub static __REPLACE: [::std::sync::atomic::AtomicPtr<()>; {cells}] = [const {{ ::std::sync::atomic::AtomicPtr::new(::std::ptr::null_mut()) }}; {cells}];\n"
        ));
        // The game crate arms the crates below it too: it is where the points are sealed.
        let below: Vec<String> = crates
            .iter()
            .filter(|other| !krate.marked_only && other.marked_only)
            .map(|other| format!("::{}::__fnr_arm", other.name))
            .collect();
        let calls: String = armers
            .iter()
            .chain(&below)
            .map(|path| format!(" {path}();"))
            .collect();
        root_text.push_str(&format!(
            "/// Installs the dispatcher of each injection point of this crate that a mod listens at as its function's entry: called once, after the points are sealed.\n#[doc(hidden)]\npub fn __fnr_arm() {{{calls} }}\n"
        ));
        texts.insert(&krate.lib, root_text);
        for (path, text) in texts {
            fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        staged.functions += count;
        staged.crates += 1;
    }
    if let Some(((origin, path), _)) = list.iter().zip(&used).find(|(_, used)| !**used) {
        return Err(format!(
            "{origin}: {path}: no free function or inherent method every build compiles that can be replaced"
        ));
    }
    let below: Vec<&Path> = crates
        .iter()
        .filter(|krate| krate.marked_only)
        .map(|krate| krate.package.as_path())
        .collect();
    arm_dependencies(root, &game, &below)?;
    Ok(staged)
}

/// `path` with its `..` and `.` resolved, as written: the staged copy has no links to follow.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// A package's manifest, parsed.
pub(crate) fn manifest(package: &Path) -> Result<toml::Table, String> {
    let path = package.join("Cargo.toml");
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether `package` is built on `game` through its path dependencies (normal and build ones).
fn built_on(package: &Path, game: &Path) -> Result<bool, String> {
    let game = lexical(game);
    let mut pending = vec![lexical(package)];
    let mut seen = BTreeSet::new();
    while let Some(next) = pending.pop() {
        if !seen.insert(next.clone()) {
            continue;
        }
        let cargo = manifest(&next)?;
        for kind in ["dependencies", "build-dependencies"] {
            let Some(table) = cargo.get(kind).and_then(toml::Value::as_table) else {
                continue;
            };
            for dependency in table.values() {
                if let Some(path) = dependency.get("path").and_then(toml::Value::as_str) {
                    let folder = lexical(&next.join(path));
                    if folder == game {
                        return Ok(true);
                    }
                    pending.push(folder);
                }
            }
        }
    }
    Ok(false)
}

/// Give the staged game crate a path dependency on each crate it arms, in its manifest and its
/// lock entry: its source manifest needn't name a crate it reaches only through slots.
fn arm_dependencies(root: &Path, game: &Path, armed: &[&Path]) -> Result<(), String> {
    if armed.is_empty() {
        return Ok(());
    }
    let mut cargo = manifest(game)?;
    let game_name = cargo
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("{}: no package name", game.display()))?
        .to_owned();
    let dependencies = cargo
        .entry("dependencies")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("[dependencies] is not a table")?;
    let mut added = Vec::new();
    for package in armed {
        let name = manifest(package)?
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("{}: no package name", package.display()))?
            .to_owned();
        // Kept under whatever name the game crate already gives it.
        let present = dependencies.iter().any(|(key, dependency)| {
            *key == name
                || dependency.get("package").and_then(toml::Value::as_str) == Some(name.as_str())
        });
        if present {
            continue;
        }
        let folder = package
            .file_name()
            .ok_or_else(|| format!("{}: no folder name", package.display()))?
            .to_string_lossy()
            .into_owned();
        dependencies.insert(
            name.clone(),
            toml::Value::Table(toml::Table::from_iter([(
                "path".to_owned(),
                format!("../{folder}").into(),
            )])),
        );
        added.push(name);
    }
    if added.is_empty() {
        return Ok(());
    }
    let lock_path = root.join("Cargo.lock");
    // A Windows checkout has CRLF line endings; Cargo reads either, and the entries match on LF.
    let lock = fs::read_to_string(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?
        .replace("\r\n", "\n");
    let added: Vec<&str> = added.iter().map(String::as_str).collect();
    let lock = crate::points::locked_with(&lock, &game_name, &added)
        .ok_or_else(|| format!("Cargo.lock has no entry for {game_name}"))?;
    let path = game.join("Cargo.toml");
    fs::write(&path, toml::to_string(&cargo).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    fs::write(&lock_path, lock).map_err(|e| format!("{}: {e}", lock_path.display()))
}

/// `text` with the head check, reader and installer of each of `found`, numbering their cells
/// from `cells`, with each `replaced!(f)` of a function it stages answered from `f`'s entry, and
/// with `arm_points!()` calling the crate's `__fnr_arm`. A function that is an injection point
/// gets its cell number in its `#[point]` instead of a head check, and its armer's path joins
/// `armers`.
/// None when the file has nothing to stage.
fn rewrite(
    path: &Path,
    text: &str,
    found: &[Found<'_>],
    cells: &mut usize,
    armers: &mut Vec<String>,
) -> Result<Option<String>, String> {
    if found.is_empty() {
        return Ok(None);
    }
    let lines: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let at = |position| source::offset(text, &lines, position);
    // Replacements of byte ranges (an insertion is an empty range), applied in order.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut names = BTreeSet::new();
    let mut free = BTreeMap::new();
    for function in found {
        // A point's function has two cells: its entry, which its head loads, and behind it the
        // copy its dispatcher runs in place of the body.
        let point = function.point();
        let slot = *cells;
        *cells += if point.is_some() { 2 } else { 1 };
        let name = source::unraw(&function.sig.ident);
        let installer = installer(function.trait_name, &function.sig.ident);
        let reader = quote::format_ident!("__fnr_read_slot_{slot}");
        if function.owner.is_none() {
            free.insert(name.clone(), reader.to_string());
        }
        let scope = match function.owner {
            Some(block) => format!("{}::{installer}", source::owner(&block.self_ty)),
            None => name.clone(),
        };
        if !names.insert((function.module.clone(), scope)) {
            return Err(format!(
                "{}: {} is declared twice in one module: one of them can't be replaced",
                path.display(),
                function.target()
            ));
        }
        // The copy is called with the function's own arguments: a parameter bound by a pattern
        // is taken by a plain name first, and bound as written after the check.
        let mut arguments = Vec::new();
        let mut rebind = String::new();
        // A point's own expansion names the parameters and writes the head check.
        let plain_inputs = function
            .sig
            .inputs
            .iter()
            .enumerate()
            .filter(|_| point.is_none());
        for (index, input) in plain_inputs {
            match input {
                FnArg::Receiver(_) => arguments.push("self".to_owned()),
                FnArg::Typed(typed) => match &*typed.pat {
                    Pat::Ident(pat) if pat.by_ref.is_none() && pat.subpat.is_none() => {
                        arguments.push(pat.ident.to_string());
                    }
                    pat => {
                        let plain = format!("__fnr_p{index}");
                        let span = syn::spanned::Spanned::span(pat);
                        let (start, end) = (at(span.start()), at(span.end()));
                        if !matches!(pat, Pat::Wild(_)) {
                            rebind.push_str(&format!(" let {} = {plain};", &text[start..end]));
                        }
                        edits.push((start, end, plain.clone()));
                        arguments.push(plain);
                    }
                },
            }
        }
        let call = if function.owner.is_some() {
            format!("Self::{reader}()")
        } else {
            format!("{reader}()")
        };
        match point {
            Some(list) => {
                let open = at(list.delimiter.span().open().end());
                edits.push((open, open, format!("slot = {slot}, ")));
            }
            None => {
                let open = at(function.body.brace_token.span.open().end());
                edits.push((
                    open,
                    open,
                    format!(
                        " if let ::std::option::Option::Some(__fnr) = {call} {{ return __fnr({}); }}{rebind}",
                        arguments.join(", ")
                    ),
                ));
            }
        }
        let ty = match function.owner.and_then(|block| block.trait_.as_ref()) {
            Some((_, path, _)) if function.trait_name.is_some() => {
                pointer_type(&qualified(function.sig, path))
            }
            _ => pointer_type(function.sig),
        };
        let read = |reader: &syn::Ident, cell: usize| {
            quote! {
                #[doc(hidden)]
                #[inline(always)]
                #[allow(unsafe_code, dead_code, clippy::all)]
                pub fn #reader() -> ::std::option::Option<#ty> {
                    let installed = crate::__REPLACE[#cell].load(::std::sync::atomic::Ordering::Relaxed);
                    if installed.is_null() {
                        ::std::option::Option::None
                    } else {
                        // SAFETY: only this function's installer and its point's dispatcher, both
                        // of this type, are stored here.
                        ::std::option::Option::Some(unsafe { ::std::mem::transmute::<*mut (), #ty>(installed) })
                    }
                }
            }
        };
        let install = |cell: usize| {
            quote! {
                crate::__REPLACE[#cell]
                    .compare_exchange(
                        ::std::ptr::null_mut(),
                        replacement as *mut (),
                        ::std::sync::atomic::Ordering::Release,
                        ::std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_ok()
            }
        };
        let entry = read(&reader, slot);
        let items = if point.is_some() {
            // The copy goes behind the entry, where the point's dispatcher finds it, and into
            // the entry itself unless the dispatcher is there already.
            let body = slot + 1;
            let body_reader = quote::format_ident!("__fnr_read_body_{slot}");
            let (behind, body_read, direct) =
                (install(body), read(&body_reader, body), install(slot));
            quote! {
                #entry
                #body_read
                #[doc(hidden)]
                #[inline]
                #[allow(dead_code, non_snake_case, clippy::all)]
                pub fn #installer(replacement: #ty) -> bool {
                    let installed = #behind;
                    if installed {
                        let _ = #direct;
                    }
                    installed
                }
            }
        } else {
            let direct = install(slot);
            quote! {
                #entry
                #[doc(hidden)]
                #[inline]
                #[allow(dead_code, non_snake_case, clippy::all)]
                pub fn #installer(replacement: #ty) -> bool {
                    #direct
                }
            }
        };
        if point.is_some() {
            let armer = quote::format_ident!("__pointa_{name}");
            let arm = quote::format_ident!("__fnr_arm_{slot}");
            let call = match function.owner {
                Some(block) => {
                    let (generics, _, clause) = block.generics.split_for_impl();
                    let ty = &block.self_ty;
                    quote!(#[allow(clippy::all)] pub(crate) fn #arm #generics () #clause { <#ty>::#armer() })
                }
                None => quote!(#[allow(clippy::all)] pub(crate) fn #arm() { #armer() }),
            };
            let end = at(function.end);
            edits.push((end, end, format!(" {call}")));
            let module = function
                .module
                .split_once("::")
                .map_or(String::new(), |(_, rest)| format!("{rest}::"));
            armers.push(format!("crate::{module}{arm}"));
        }
        let items = match function.owner {
            Some(block) => {
                let (generics, _, clause) = block.generics.split_for_impl();
                let ty = &block.self_ty;
                quote!(impl #generics #ty #clause { #items })
            }
            None => items,
        };
        let end = at(function.end);
        edits.push((end, end, format!(" {items}")));
    }
    // The placeholder macro has no callers after its invocations are rewritten. Remove its
    // declaration too, keeping line endings so source locations still match the pinned tree.
    let parsed = syn::parse_file(text).map_err(|e| format!("{}: {e}", path.display()))?;
    for item in parsed.items {
        if let Item::Macro(item) = item
            && item.mac.path.is_ident("macro_rules")
            && item
                .ident
                .as_ref()
                .is_some_and(|name| name == "replaced" || name == "arm_points")
        {
            let span = syn::spanned::Spanned::span(&item);
            let (start, end) = (at(span.start()), at(span.end()));
            edits.push((
                start,
                end,
                text[start..end].chars().filter(|c| *c == '\n').collect(),
            ));
        }
    }
    // `replaced!(f)`: whether a copy of the free function `f` of this module is installed.
    let mut from = 0;
    while let Some(start) = text[from..].find("replaced!(").map(|i| i + from) {
        let inner = start + "replaced!(".len();
        let Some(close) = text[inner..].find(')').map(|i| i + inner) else {
            break;
        };
        from = close;
        let name = text[inner..close].trim();
        let Some(reader) = free.get(name) else {
            return Err(format!(
                "{}: replaced!({name}): no replaceable free function of that name in this file",
                path.display()
            ));
        };
        edits.push((start, close + 1, format!("{reader}().is_some()")));
    }
    if let Some(start) = text.find("arm_points!()") {
        edits.push((
            start,
            start + "arm_points!()".len(),
            "crate::__fnr_arm()".into(),
        ));
    }
    // At one offset, a replacement of a range before an insertion there, each in pushed order.
    edits.sort_by_key(|(start, end, _)| (*start, *end == *start));
    let mut out =
        String::with_capacity(text.len() + edits.iter().map(|e| e.2.len()).sum::<usize>());
    let mut from = 0;
    for (start, end, replacement) in edits {
        if start < from {
            return Err(format!("{}: overlapping staging edits", path.display()));
        }
        out.push_str(&text[from..start]);
        out.push_str(&replacement);
        from = end;
    }
    out.push_str(&text[from..]);
    debug_assert_eq!(out.matches('\n').count(), text.matches('\n').count());
    Ok(Some(out))
}

/// The names of a file's replaceable functions, as targets: what the patch generator and tests
/// compare against.
pub fn targets(
    module: &str,
    file: &syn::File,
    traits: &BTreeSet<String>,
    types: &OwnerTypes,
) -> Vec<String> {
    let mut found = Vec::new();
    functions(module, &file.items, traits, types, &mut found);
    found.iter().map(Found::target).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged(source: &str) -> String {
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &BTreeSet::new(),
            &OwnerTypes::default(),
            &mut found,
        );
        let mut count = 0;
        rewrite(
            Path::new("k.rs"),
            source,
            &found,
            &mut count,
            &mut Vec::new(),
        )
        .unwrap()
        .unwrap_or_else(|| source.to_owned())
    }

    fn eligible_fn(source: &str) -> bool {
        let function: syn::ItemFn = syn::parse_str(source).unwrap();
        eligible(&function.attrs, &function.sig)
    }

    #[test]
    fn hook_functions_are_replaceable_and_single_instance_rules_refuse() {
        assert!(eligible_fn("fn native_tick(&mut self) {}"));
        assert!(eligible_fn(
            "fn borrow<'a>(&'a self, key: &str) -> &'a str { key }"
        ));
        for refused in [
            "const fn f() {}",
            "async fn f() {}",
            "unsafe fn f() {}",
            "fn f<T>(value: T) {}",
            "fn f<const N: usize>() {}",
            "fn f(value: impl Clone) {}",
            "fn f() -> impl Clone {}",
            "fn f(self) {}",
            "fn f(self: Box<Self>) {}",
            "fn f() -> ! { loop {} }",
            "#[inline(always)] fn f() {}",
            "#[track_caller] fn f() {}",
            "#[cfg(unix)] fn f() {}",
        ] {
            assert!(!eligible_fn(refused), "{refused}");
        }
    }

    #[test]
    fn a_method_checks_its_slot_first_and_forwards_its_own_arguments() {
        let out = staged(
            "impl Game<'_> {\n    fn next(&mut self, (a, b): (u32, u32), mut c: u8, _: bool) -> u32 {\n        a + b\n    }\n}\n",
        );
        assert!(
            out.contains(
                "fn next(&mut self, __fnr_p1: (u32, u32), mut c: u8, __fnr_p3: bool) -> u32 { if let ::std::option::Option::Some(__fnr) = Self::__fnr_read_slot_0() { return __fnr(self, __fnr_p1, c, __fnr_p3); } let (a, b) = __fnr_p1;\n"
            ),
            "{out}"
        );
        // The reader and installer sit in an `impl` of the same type, after the method's own.
        let file = syn::parse_file(&out).unwrap();
        let Item::Impl(added) = &file.items[1] else {
            panic!("{out}")
        };
        assert_eq!(source::owner(&added.self_ty), "Game<'_>");
        let names: Vec<_> = added
            .items
            .iter()
            .map(|item| match item {
                ImplItem::Fn(function) => function.sig.ident.to_string(),
                _ => panic!("{out}"),
            })
            .collect();
        assert_eq!(names, ["__fnr_read_slot_0", "__fnr_next"]);
        assert_eq!(out.lines().count(), 5);
    }

    #[test]
    fn the_pointer_type_binds_the_functions_lifetimes_and_the_receivers_borrow() {
        let method: syn::ImplItemFn =
            syn::parse_str("fn name<'a>(&self, key: &'a str, other: &Self) -> &str { key }")
                .unwrap();
        assert_eq!(
            pointer_type(&method.sig).to_string(),
            quote!(
                for<'__fnr_l0, '__fnr_s> fn(&'__fnr_s Self, &'__fnr_l0 str, &Self) -> &'__fnr_s str
            )
            .to_string()
        );
        let free: syn::ItemFn = syn::parse_str("fn interval(rate: f32) -> i64 { 0 }").unwrap();
        assert_eq!(
            pointer_type(&free.sig).to_string(),
            quote!(fn(f32) -> i64).to_string()
        );
    }

    #[test]
    fn inventory_partitions_definitions_by_the_actual_eligibility_rules() {
        let file = syn::parse_file("fn free() {} const fn constant() {} fn generic<T>() {} struct T; trait Owned { fn required(&self); fn default(&self) {} } impl Owned for T { fn required(&self) {} } #[cfg(test)] mod tests { fn helper() {} } impl T { fn consume(self) {} }").unwrap();
        let traits = BTreeSet::from(["Owned".to_owned()]);
        let mut types = OwnerTypes::default();
        types.declare("k", &file.items);
        let mut counts = BTreeMap::new();
        inventory("k", &file.items, &traits, &types, None, &mut counts);
        assert_eq!(counts.values().sum::<usize>(), 7);
        assert_eq!(
            counts,
            BTreeMap::from([
                ("eligible", 2),
                ("const", 1),
                ("function type/const generics", 1),
                ("trait default body", 1),
                ("conditional/test module", 1),
                ("by-value/typed receiver", 1),
            ])
        );
        let mut found = Vec::new();
        functions("fixture", &file.items, &traits, &types, &mut found);
        assert_eq!(found.len(), counts["eligible"]);
    }

    #[test]
    #[ignore = "daily"]
    fn trait_installers_compile_with_warnings_denied() {
        let source = "pub struct Example; pub trait GameTrait { fn value(&self) -> u32; } impl GameTrait for Example { fn value(&self) -> u32 { 7 } }";
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &BTreeSet::from(["GameTrait".to_owned()]),
            &{
                let mut types = OwnerTypes::default();
                types.declare("k", &file.items);
                types
            },
            &mut found,
        );
        let mut count = 0;
        let mut out = rewrite(
            Path::new("k.rs"),
            source,
            &found,
            &mut count,
            &mut Vec::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(count, 1);
        out.push_str(" pub static __REPLACE: [std::sync::atomic::AtomicPtr<()>; 1] = [std::sync::atomic::AtomicPtr::new(std::ptr::null_mut())];");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("fnr-trait-lint-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("fixture.rs");
        fs::write(&input, out).unwrap();
        let result = std::process::Command::new("rustc")
            .args([
                "--edition=2024",
                "--crate-type=lib",
                "--emit=metadata",
                "-Dwarnings",
            ])
            .arg(&input)
            .arg("--out-dir")
            .arg(&dir)
            .output()
            .unwrap();
        fs::remove_dir_all(dir).unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn reader_names_do_not_collide_with_get_function_installers() {
        let out = staged(
            "struct Game; impl Game { fn projectile(&self) {} fn get_projectile(&self) {} }",
        );
        let file = syn::parse_file(&out).unwrap();
        let mut names = BTreeSet::new();
        for item in file.items {
            if let Item::Impl(block) = item {
                for item in block.items {
                    if let ImplItem::Fn(method) = item {
                        assert!(
                            names.insert(method.sig.ident.to_string()),
                            "duplicate method: {}",
                            method.sig.ident
                        );
                    }
                }
            }
        }
        assert!(names.contains("__fnr_get_projectile"));
        assert!(names.contains("__fnr_read_slot_0"));
    }

    #[test]
    fn slots_are_numbered_per_crate_and_replaced_reads_the_slot() {
        let source = "macro_rules! replaced { ($name:ident) => { false }; }\npub fn world_drawn(view: &mut View) { let _ = view; }\npub fn hooked() -> bool { replaced!(world_drawn) }\n";
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &BTreeSet::new(),
            &OwnerTypes::default(),
            &mut found,
        );
        let mut count = 7;
        let out = rewrite(
            Path::new("k.rs"),
            source,
            &found,
            &mut count,
            &mut Vec::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(count, 9);
        assert!(!out.contains("macro_rules!"));
        assert_eq!(source.lines().count(), out.lines().count());
        assert!(out.contains("crate :: __REPLACE [7usize]"), "{out}");
        assert!(out.contains("crate :: __REPLACE [8usize]"), "{out}");
        assert!(
            out.contains("{ if let ::std::option::Option::Some(__fnr) = __fnr_read_slot_8() { return __fnr(); } __fnr_read_slot_7().is_some() }"),
            "{out}"
        );
        syn::parse_file(&out).unwrap();
    }

    #[test]
    fn replaced_of_a_function_the_file_does_not_stage_is_refused() {
        let source = "pub fn hooked() -> bool { replaced!(elsewhere) }\n";
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &BTreeSet::new(),
            &OwnerTypes::default(),
            &mut found,
        );
        let error =
            rewrite(Path::new("k.rs"), source, &found, &mut 0, &mut Vec::new()).unwrap_err();
        assert!(error.contains("replaced!(elsewhere)"), "{error}");
    }

    #[test]
    fn other_crates_traits_generic_impls_and_test_modules_get_no_slot() {
        let file = syn::parse_file(
            "impl Clone for A { fn clone(&self) -> Self { A } }\nimpl<T> B<T> { fn f(&self) {} }\n#[cfg(test)] mod tests { fn t() {} }\nmod inner { fn kept() {} }\nfn free() {}\n",
        )
        .unwrap();
        assert_eq!(
            targets(
                "k",
                &file,
                &BTreeSet::from(["Goal".to_owned()]),
                &OwnerTypes::default()
            ),
            ["k::inner::kept", "k::free"]
        );
    }

    #[test]
    fn a_method_of_the_games_own_trait_gets_a_slot_on_its_type() {
        let source = "use engine::Style; struct Wander; struct Raw; mod other { struct Style; }\nimpl Goal for Style { fn start(&mut self) {} } impl Goal for engine::Raw { fn start(&mut self) {} } impl Goal for Wander { fn start(&mut self, mob: &Mob) -> bool { true } fn kind(&self) -> Self::Kind { Kind } fn sized(&self) -> [u8; Self::N] { [0; Self::N] } }\nimpl Goal<u8> for Wander { fn start(&mut self) {} }\nunsafe impl Goal for Raw { fn start(&mut self) {} }\nimpl Goal for Vec3 { fn start(&mut self) {} }\n";
        let traits = BTreeSet::from(["Goal".to_owned()]);
        let file = syn::parse_file(source).unwrap();
        let mut types = OwnerTypes::default();
        types.declare("k", &file.items);
        // A signature naming the trait's associated constant, a trait written with arguments, an
        // unsafe impl and another crate's type (imported under a name the crate also declares,
        // written by path, or not declared here at all) are left as the game wrote them.
        assert_eq!(
            targets("k", &file, &traits, &types),
            ["k::<Wander as Goal>::start", "k::<Wander as Goal>::kind"]
        );
        let mut found = Vec::new();
        functions("k", &file.items, &traits, &types, &mut found);
        let out = rewrite(Path::new("k.rs"), source, &found, &mut 0, &mut Vec::new())
            .unwrap()
            .unwrap();
        assert!(
            out.contains("{ if let ::std::option::Option::Some(__fnr) = Self::__fnr_read_slot_0() { return __fnr(self, mob); } true }"),
            "{out}"
        );
        let staged = syn::parse_file(&out).unwrap();
        let Some(Item::Impl(added)) = staged.items.get(7) else {
            panic!("{out}")
        };
        assert!(added.trait_.is_none());
        assert!(out.contains("pub fn __fnr_as_Goal_start"), "{out}");
        // The trait's associated type is named through the trait outside its `impl`.
        assert!(out.contains("-> < Self as Goal > :: Kind"), "{out}");
    }

    #[test]
    fn a_trait_impl_in_another_file_from_its_type_gets_its_slot() {
        let traits = BTreeSet::from(["LevelAccess".to_owned()]);
        let declares = syn::parse_file("pub struct GameLevel;").unwrap();
        let implements = syn::parse_file(
            "impl LevelAccess for GameLevel { fn block_state(&self) -> u32 { 0 } }",
        )
        .unwrap();
        let mut types = OwnerTypes::default();
        types.declare("k::levels", &declares.items);
        types.declare("k::level_access", &implements.items);
        // The crate's only type of that name.
        assert_eq!(
            targets("k::level_access", &implements, &traits, &types),
            ["k::level_access::<GameLevel as LevelAccess>::block_state"]
        );
        let Item::Impl(block) = &implements.items[0] else {
            panic!("an impl")
        };
        let foreign = foreign_imports(&implements.items);
        let here = "k::level_access";
        assert_eq!(trait_refusal(here, block, &traits, &types, &foreign), None);
        // A second module declaring the name, and no `use` in the `impl`'s module names either:
        // a bare `GameLevel` no longer says which type.
        types.declare("k::other", &declares.items);
        assert!(targets("k::level_access", &implements, &traits, &types).is_empty());
        assert_eq!(
            trait_refusal(here, block, &traits, &types, &foreign).unwrap(),
            "its trait implementation has no replacement installer: owner type name `GameLevel` is ambiguous in the crate: more than one module declares it, and the `impl`'s module neither declares it nor imports it from one of them"
        );
    }

    /// The targets of `impl LevelAccess for GameLevel` in `k::game::level_access`, a child of
    /// `k::game`, whose file starts with `import`; each of `others` is a module and its file.
    fn level_access_targets(others: &[(&str, &str)], import: &str) -> Vec<String> {
        let traits = BTreeSet::from(["LevelAccess".to_owned()]);
        let mut types = OwnerTypes::default();
        for (module, text) in others {
            types.declare(module, &syn::parse_file(text).unwrap().items);
        }
        let file = syn::parse_file(&format!(
            "{import} impl LevelAccess for GameLevel {{ fn block_state(&self) -> u32 {{ 0 }} }}"
        ))
        .unwrap();
        types.declare("k::game::level_access", &file.items);
        targets("k::game::level_access", &file, &traits, &types)
    }

    const KEPT: [&str; 1] = ["k::game::level_access::<GameLevel as LevelAccess>::block_state"];

    /// The game's own shape: `GameLevel` in the `game` module and in another one, the `impl`
    /// in a child of `game` that takes its parent's names.
    #[test]
    fn a_use_of_the_declaring_module_settles_a_name_two_modules_declare() {
        let twice = [
            ("k::game", "struct GameLevel;"),
            ("k::game::natural_spawning", "struct GameLevel;"),
        ];
        for import in [
            "use super::*;",
            "use super::GameLevel;",
            "use crate::game::{GameLevel, Other};",
            "use crate::game::natural_spawning::GameLevel;",
        ] {
            assert_eq!(level_access_targets(&twice, import), KEPT, "{import}");
        }
        // Renamed from a name that is no type there, a name its module does not hold, or brought
        // in by two globs that both declare it: nothing says which.
        for import in [
            "use super::Level as GameLevel;",
            "use crate::elsewhere::GameLevel;",
            "use super::*; use super::natural_spawning::*;",
        ] {
            assert!(level_access_targets(&twice, import).is_empty(), "{import}");
        }
        // One module reached through two globs is one answer.
        let through = [
            (
                "k::game",
                "struct GameLevel; mod other { struct GameLevel; }",
            ),
            ("k::game::prelude", "use super::*;"),
        ];
        assert_eq!(
            level_access_targets(&through, "use super::*; use super::prelude::*;"),
            KEPT
        );
    }

    /// A name the parent takes from another crate reaches the child through its glob: the
    /// crate's own type of that name, declared elsewhere, is not what the `impl` names.
    #[test]
    fn a_foreign_name_inherited_through_a_glob_is_refused() {
        for parent in [
            "use engine::GameLevel;",
            "use engine::Level as GameLevel;",
            "use engine::*;",
        ] {
            let crate_ = [("k::game", parent), ("k::levels", "struct GameLevel;")];
            assert!(
                level_access_targets(&crate_, "use super::*;").is_empty(),
                "{parent}"
            );
        }
        // The same glob beside the parent's own type: the type is the parent's.
        let own = [("k::game", "use engine::*; struct GameLevel;")];
        assert_eq!(level_access_targets(&own, "use super::*;"), KEPT);
        // A glob of an enum's variants, or of a child module, brings in no other crate's name.
        let variants = [
            (
                "k::game",
                "enum Kind { A } use Kind::*; mod child {} use child::*;",
            ),
            ("k::levels", "struct GameLevel;"),
        ];
        assert_eq!(level_access_targets(&variants, "use super::*;"), KEPT);
    }

    /// A renamed import is the type it renames: followed when that is a type of the crate,
    /// refused when it is another crate's or leads nowhere, and never dropped for a type that
    /// happens to have the new name.
    #[test]
    fn a_renamed_import_is_followed_to_the_type_it_renames() {
        let own = [("k::a", "struct Level;"), ("k::b", "struct GameLevel;")];
        assert_eq!(
            level_access_targets(&own, "use crate::a::Level as GameLevel;"),
            KEPT
        );
        for (a, import) in [
            // `k::a` takes `Level` from another crate, by name and by glob.
            ("use engine::Level;", "use crate::a::Level as GameLevel;"),
            ("use engine::*;", "use crate::a::Level as GameLevel;"),
            // Renamed straight from another crate, in the parent.
            ("", "use engine::Level as GameLevel;"),
        ] {
            let crate_ = [("k::a", a), ("k::b", "struct GameLevel;")];
            assert!(
                level_access_targets(&crate_, import).is_empty(),
                "{a} {import}"
            );
        }
        // Inherited through the parent's glob, the renamed name is still not `k::b`'s type.
        let inherited = [
            ("k::game", "use engine::Level as GameLevel;"),
            ("k::b", "struct GameLevel;"),
        ];
        assert!(level_access_targets(&inherited, "use super::*;").is_empty());
    }

    /// `mod_api`'s `PartRegistrar` shape: the type in a child module, re-exported by its parent
    /// through a path written from the child's bare name, and the `impl`s in sibling files that
    /// take the parent's names.
    #[test]
    fn a_parents_re_export_of_its_childs_type_reaches_a_sibling() {
        for re_export in [
            "pub use shared::GameLevel;",
            "pub use shared::*;",
            "pub use self::shared::GameLevel;",
            "pub use shared::{GameLevel as Level, GameLevel};",
        ] {
            let crate_ = [
                ("k::game::shared", "pub struct GameLevel;"),
                ("k::game", re_export),
                ("k::other", "struct GameLevel;"),
            ];
            assert_eq!(
                level_access_targets(&crate_, "use super::*;"),
                KEPT,
                "{re_export}"
            );
        }
        // Re-exported under the name: the sibling's `GameLevel` is the child's `Shared`, a type of
        // the crate, whatever else is called `GameLevel`.
        let renamed = [
            ("k::game::shared", "pub struct Shared;"),
            ("k::game", "pub use shared::Shared as GameLevel;"),
            ("k::other", "struct GameLevel;"),
        ];
        assert_eq!(level_access_targets(&renamed, "use super::*;"), KEPT);
        // A child the parent does not have is another crate.
        let foreign = [
            ("k::game", "pub use engine_shared::GameLevel;"),
            ("k::other", "struct GameLevel;"),
        ];
        assert!(level_access_targets(&foreign, "use super::*;").is_empty());
    }

    /// A module's `use`s are its own, whichever of its files writes them.
    #[test]
    fn a_use_in_a_sibling_file_of_the_module_settles_the_name() {
        let crate_ = [
            ("k::a", "struct GameLevel;"),
            ("k::b", "struct GameLevel;"),
            ("k::game::level_access", "use crate::a::GameLevel;"),
        ];
        assert_eq!(level_access_targets(&crate_, ""), KEPT);
    }

    #[test]
    fn two_modules_types_of_one_name_each_keep_their_own_impls_slot() {
        let traits = BTreeSet::from(["Goal".to_owned()]);
        let file = syn::parse_file("struct State; impl Goal for State { fn start(&mut self) {} }")
            .unwrap();
        let mut types = OwnerTypes::default();
        for module in ["k::a", "k::b"] {
            types.declare(module, &file.items);
        }
        for module in ["k::a", "k::b"] {
            assert_eq!(
                targets(module, &file, &traits, &types),
                [format!("{module}::<State as Goal>::start")]
            );
        }
    }

    #[test]
    fn declared_traits_are_the_trait_items_of_the_crates() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("crates/horizon-low/src/deep");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("a.rs"),
            "/// Not a trait Item, only words.\npub trait Goal {}\n    pub(crate) trait Inner: Goal {}\nunsafe trait Raw {}\nfn f(x: &dyn Portrait) {}\n",
        )
        .unwrap();
        assert_eq!(
            declared_traits(root.path()).unwrap(),
            BTreeSet::from(["Goal".to_owned(), "Inner".to_owned(), "Raw".to_owned()])
        );
    }

    #[test]
    fn the_pass_gives_each_crate_one_array_and_refuses_an_ineligible_marked_function() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "crates/horizon-client/Cargo.toml",
            "[package]\nname = 'horizon-client'\n",
        );
        write(
            "crates/horizon-client/src/lib.rs",
            "#![forbid(unsafe_code)]\nmod game;\n",
        );
        write(
            "crates/horizon-client/src/game.rs",
            "impl Game<'_> { fn native_tick(&mut self) {} }\nfn free(a: u8) -> u8 { a }\n",
        );
        write(
            "crates/horizon-low/Cargo.toml",
            "[package]\nname = 'horizon-low'\n",
        );
        write(
            "crates/horizon-low/src/lib.rs",
            "#[horizon_mod_api_macros::replaceable] pub fn drawn() {}\npub fn unlisted() {}\n",
        );
        write(
            "Cargo.lock",
            "version = 4\n\n[[package]]\nname = \"horizon-client\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"horizon-low\"\nversion = \"0.1.0\"\n",
        );

        let staged = stage(root.path()).unwrap();
        assert_eq!(
            staged,
            Staged {
                functions: 3,
                files: 2,
                crates: 2,
                slots: BTreeSet::from([
                    "horizon_client::game::Game::native_tick".to_owned(),
                    "horizon_client::game::free".to_owned(),
                    "horizon_low::drawn".to_owned(),
                ]),
            }
        );
        let lib = fs::read_to_string(root.path().join("crates/horizon-client/src/lib.rs")).unwrap();
        assert!(lib.starts_with("#![deny(unsafe_code)]\n"), "{lib}");
        assert!(lib.contains("pub static __REPLACE: [::std::sync::atomic::AtomicPtr<()>; 2]"));
        let low = fs::read_to_string(root.path().join("crates/horizon-low/src/lib.rs")).unwrap();
        assert!(
            low.contains("__fnr_drawn") && !low.contains("__fnr_unlisted"),
            "{low}"
        );
        assert!(low.contains("pub static __REPLACE: [::std::sync::atomic::AtomicPtr<()>; 1]"));
        syn::parse_file(&low).unwrap();
        // The game crate arms the marked crate, so the staged copy depends on it.
        assert!(lib.contains("::horizon_low::__fnr_arm();"), "{lib}");
        let client: toml::Table = toml::from_str(
            &fs::read_to_string(root.path().join("crates/horizon-client/Cargo.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            client["dependencies"]["horizon-low"]["path"].as_str(),
            Some("../horizon-low")
        );
        let lock = fs::read_to_string(root.path().join("Cargo.lock")).unwrap();
        assert!(
            lock.contains("name = \"horizon-client\"\nversion = \"0.1.0\"\ndependencies = [\n \"horizon-low\",\n]"),
            "{lock}"
        );

        write(
            "crates/horizon-low/src/lib.rs",
            "#[horizon_mod_api_macros::replaceable] pub const fn missing() {}\n",
        );
        let error = stage(root.path()).unwrap_err();
        assert!(
            error.contains("#[horizon_mod_api_macros::replaceable] on horizon_low::missing"),
            "{error}"
        );
    }

    #[test]
    fn a_marked_crate_built_on_the_game_crate_is_refused_by_name() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "crates/horizon-client/Cargo.toml",
            "[package]\nname = 'horizon-client'\n",
        );
        write("crates/horizon-client/src/lib.rs", "mod game;\n");
        write("crates/horizon-client/src/game.rs", "");
        // Through a crate in between, as a cycle usually hides.
        write(
            "crates/horizon-high/Cargo.toml",
            "[package]\nname = 'horizon-high'\n[dependencies]\nhorizon-mid = { path = '../horizon-mid' }\n",
        );
        write(
            "crates/horizon-mid/Cargo.toml",
            "[package]\nname = 'horizon-mid'\n[build-dependencies]\nhorizon-client = { path = '../horizon-client' }\n",
        );
        write(
            "crates/horizon-high/src/lib.rs",
            "#[horizon_mod_api_macros::replaceable] pub fn drawn() {}\n",
        );
        let error = stage(root.path()).unwrap_err();
        assert!(
            error.starts_with("horizon_high: it is built on horizon-client"),
            "{error}"
        );
        // Left untouched: the refusal comes before any edit.
        assert_eq!(
            fs::read_to_string(root.path().join("crates/horizon-high/src/lib.rs")).unwrap(),
            "#[horizon_mod_api_macros::replaceable] pub fn drawn() {}\n"
        );
    }
}
