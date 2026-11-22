//! How a game internal crosses to a wasm mod (arismoko/horizon#589 W): one rule for both sides, the mod's generated
//! view and the game's generated table, read from the same `internals.json` signatures.
//!
//! - A **copy** is plain data all the way down, at most [`LEAVES`] scalars: it crosses as a value.
//! - A **handle** has fields or elements a mod walks into: it crosses as a node of a path. Its
//!   parts that are opaque are just not reachable.
//! - An **opaque** type crosses neither way: trait objects, closures, cells and locks, channels,
//!   GPU and window objects, raw pointers, generic types, and types this game doesn't list.
//!
//! Each written type resolves to a [`Shape`], whose types are spelled from the game crate's root,
//! so the game's generated code names exactly the type the signature means.
use crate::internals::Internal;
use quote::ToTokens;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use syn::{GenericArgument, PathArguments, Type};

/// The most scalars a copy holds; a bigger plain type crosses as a handle.
pub const LEAVES: usize = 16;

/// How one type crosses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Crossing {
    /// A value of this many scalars.
    Copy(usize),
    /// A node a mod reads into, field by field or element by element.
    Handle,
    /// Nothing crosses.
    Opaque,
}

/// A scalar's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scalar {
    Bool,
    Char,
    /// A signed integer, by its Rust name.
    Signed(&'static str),
    Unsigned(&'static str),
    Float(&'static str),
    /// A 128-bit integer, carried as high and low unsigned 64-bit leaves.
    Wide(&'static str),
    /// `str`: read only.
    Str,
    String,
}

/// A wrapper read through: `Box` owns its value; `Arc`, `Rc` and `Cow` share it, so it is read
/// only through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrapper {
    Box,
    Arc,
    Rc,
    Cow,
}

/// A std collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collection {
    Vec,
    VecDeque,
    HashSet,
    BTreeSet,
    HashMap,
    BTreeMap,
}

/// What a written type is, resolved.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Scalar(Scalar),
    Unit,
    Ref {
        mutable: bool,
        to: Box<Shape>,
    },
    Tuple(Vec<Shape>),
    Array(Box<Shape>, usize),
    Slice(Box<Shape>),
    Option(Box<Shape>),
    /// `Result<T, E>` (and the game's `Result<T>` alias, whose error is `Box<dyn Error>`): a
    /// two-case tagged enum, Ok = 0 and Err = 1, each with one payload field.
    Result(Box<Shape>, Box<Shape>),
    /// `dyn Error`: crosses as its `Display` text, so a `Box<dyn Error>` is read as a string and
    /// replaced by building a boxed error from one.
    DynError,
    Wrapped(Wrapper, Box<Shape>),
    /// A collection, with its written arguments (a map's key and value, and any hasher, as types
    /// from the crate's root).
    Collection(Collection, Vec<Shape>),
    /// A struct or enum this game lists, by path, with its count of lifetime parameters.
    Named(String, usize),
    /// A type that never crosses, such as a hasher argument: spelled, never walked.
    Opaque(Option<String>),
}

impl Shape {
    /// Whether a value may contain a borrow that a call-site result cannot respell safely.
    /// Lifetime-bearing names include omitted lifetimes; opaque layouts are conservatively refused.
    pub fn may_borrow(&self) -> bool {
        match self {
            Self::Ref { .. } | Self::Opaque(_) | Self::Wrapped(Wrapper::Cow, _) => true,
            Self::Named(_, lifetimes) => *lifetimes != 0,
            Self::Tuple(parts) | Self::Collection(_, parts) => parts.iter().any(Self::may_borrow),
            Self::Result(ok, err) => ok.may_borrow() || err.may_borrow(),
            Self::Array(inner, _)
            | Self::Slice(inner)
            | Self::Option(inner)
            | Self::Wrapped(_, inner) => inner.may_borrow(),
            Self::Scalar(_) | Self::Unit | Self::DynError => false,
        }
    }
}

/// Scalars by name.
const SCALARS: &[(&str, Scalar)] = &[
    ("bool", Scalar::Bool),
    ("char", Scalar::Char),
    ("i8", Scalar::Signed("i8")),
    ("i16", Scalar::Signed("i16")),
    ("i32", Scalar::Signed("i32")),
    ("i64", Scalar::Signed("i64")),
    ("i128", Scalar::Wide("i128")),
    ("isize", Scalar::Signed("isize")),
    ("u8", Scalar::Unsigned("u8")),
    ("u16", Scalar::Unsigned("u16")),
    ("u32", Scalar::Unsigned("u32")),
    ("u64", Scalar::Unsigned("u64")),
    ("u128", Scalar::Wide("u128")),
    ("usize", Scalar::Unsigned("usize")),
    ("f32", Scalar::Float("f32")),
    ("f64", Scalar::Float("f64")),
    ("str", Scalar::Str),
    ("String", Scalar::String),
];

const WRAPPERS: &[(&str, Wrapper)] = &[
    ("Box", Wrapper::Box),
    ("Arc", Wrapper::Arc),
    ("Rc", Wrapper::Rc),
    ("Cow", Wrapper::Cow),
];

const COLLECTIONS: &[(&str, Collection)] = &[
    ("Vec", Collection::Vec),
    ("VecDeque", Collection::VecDeque),
    ("HashSet", Collection::HashSet),
    ("BTreeSet", Collection::BTreeSet),
    ("HashMap", Collection::HashMap),
    ("BTreeMap", Collection::BTreeMap),
];

/// Type aliases of `Result<T, Box<dyn Error>>`, by resolved path.
const ERROR_RESULTS: &[&str] = &["horizon_client_core::Result", "horizon_client::Result"];

/// Crates whose types never cross: the GPU, the window, input devices, audio. Only the
/// [`CROSSING_FOREIGN`] types of them do.
const OPAQUE_CRATES: &[&str] = &["wgpu", "winit", "gilrs", "cpal", "rodio", "glyphon"];

/// How a [`CROSSING_FOREIGN`] type's definition is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foreign {
    /// A struct or enum, as written.
    Layout,
    /// A `bitflags!` struct, which has no field to read: it crosses as its bits, a one-field
    /// struct of the flags' integer (`bits()` / `from_bits_retain`).
    Bits,
}

/// The types of crates outside the workspace that cross by copy (arismoko/horizon#1409): the key and mouse
/// state the input internals take. Their definitions are read from the source Cargo locked for
/// the game, and listed in `internals.json` like the workspace's.
pub const CROSSING_FOREIGN: &[(&str, Foreign)] = &[
    ("winit::keyboard::KeyCode", Foreign::Layout),
    ("winit::keyboard::PhysicalKey", Foreign::Layout),
    ("winit::keyboard::NativeKeyCode", Foreign::Layout),
    ("winit::keyboard::KeyLocation", Foreign::Layout),
    ("winit::keyboard::ModifiersState", Foreign::Bits),
    ("winit::event::ElementState", Foreign::Layout),
    ("winit::event::MouseButton", Foreign::Layout),
    ("winit::event_loop::ControlFlow", Foreign::Layout),
];

/// How the [`CROSSING_FOREIGN`] type at `path` is read, if it is one.
pub fn foreign(path: &str) -> Option<Foreign> {
    CROSSING_FOREIGN
        .iter()
        .find(|(listed, _)| *listed == path)
        .map(|(_, foreign)| *foreign)
}

/// The checked-in [`CROSSING_FOREIGN`] layouts, written by `cargo run -p xtask -- mod-api` from
/// the locked source. Staging also runs where Cargo may not be yet, so it reads this file.
const CROSSING_FOREIGN_FILE: &str = "crates/horizon-mod-api-gen/crossing-foreign.json";

/// One [`CROSSING_FOREIGN`] type's layout, as its crate's source declares it.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct ForeignLayout {
    path: String,
    kind: String,
    signature: String,
}

/// The [`CROSSING_FOREIGN`] entries of `internals.json`, from the checked-in layouts.
pub fn foreign_internals() -> Vec<Internal> {
    let layouts: Vec<ForeignLayout> =
        serde_json::from_str(include_str!("../crossing-foreign.json")).expect("foreign layouts");
    layouts
        .into_iter()
        .map(|layout| Internal {
            path: layout.path,
            kind: if layout.kind == "enum" {
                "enum"
            } else {
                "struct"
            },
            cfg: String::new(),
            hash: crate::internals::signature_hash(&layout.signature),
            signature: layout.signature,
            shape_only: true,
            opaque_reason: None,
            refs: BTreeMap::new(),
        })
        .collect()
}

/// Write the [`CROSSING_FOREIGN`] layouts of `root`'s checkout, read from the source of each
/// crate as the game crate resolves it in the lock file.
pub fn write_foreign_layouts(root: &Path) -> Result<(), String> {
    let path = root.join(CROSSING_FOREIGN_FILE);
    std::fs::write(&path, locked_foreign_layouts(root)?)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The text of [`CROSSING_FOREIGN_FILE`] that `root`'s locked source gives.
fn locked_foreign_layouts(root: &Path) -> Result<String, String> {
    let metadata = cargo_metadata(root)?;
    let layouts = read_foreign(CROSSING_FOREIGN, |krate| locked_lib(&metadata, krate))?;
    serde_json::to_string_pretty(&layouts)
        .map(|text| text + "\n")
        .map_err(|e| e.to_string())
}

fn cargo_metadata(root: &Path) -> Result<serde_json::Value, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = std::process::Command::new(cargo)
        .current_dir(root)
        .args([
            "metadata",
            "--format-version=1",
            "--locked",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("cargo metadata: {e}"))
}

/// The library root of the game crate's dependency `krate`, from `cargo metadata`.
fn locked_lib(metadata: &serde_json::Value, krate: &str) -> Result<PathBuf, String> {
    let packages = metadata["packages"].as_array().into_iter().flatten();
    let package =
        |pick: &dyn Fn(&serde_json::Value) -> bool| packages.clone().find(|package| pick(package));
    let game = package(&|p| p["name"] == "horizon-client").ok_or("no horizon-client package")?;
    let id = metadata["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|node| node["id"] == game["id"])
        .and_then(|node| node["deps"].as_array())
        .into_iter()
        .flatten()
        .find(|dep| dep["name"] == krate)
        .map(|dep| &dep["pkg"])
        .ok_or_else(|| format!("{krate}: not a dependency of horizon-client"))?;
    package(&|p| p["id"] == *id)
        .and_then(|p| p["targets"].as_array())
        .into_iter()
        .flatten()
        .find(|target| {
            target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "lib"))
        })
        .and_then(|target| target["src_path"].as_str())
        .map(PathBuf::from)
        .ok_or_else(|| format!("{krate}: no library in the locked package"))
}

/// The `listed` types' layouts, each read from its crate's library root at `lib(crate)`. A
/// listed path its source no longer has is an error naming it.
fn read_foreign(
    listed: &[(&str, Foreign)],
    lib: impl Fn(&str) -> Result<PathBuf, String>,
) -> Result<Vec<ForeignLayout>, String> {
    let mut libs = BTreeMap::new();
    let mut layouts = Vec::new();
    for (path, foreign) in listed {
        let mut segments: Vec<&str> = path.split("::").collect();
        let name = segments.pop().expect("a listed path");
        let krate = segments.remove(0);
        if !libs.contains_key(krate) {
            libs.insert(krate, lib(krate)?);
        }
        let stale = |why: &str| format!("{path}: {why} in the locked {krate} source");
        let items = module_items(&libs[krate], &segments).map_err(|e| stale(&e))?;
        let item = match foreign {
            Foreign::Layout => items.into_iter().find(|item| match item {
                syn::Item::Struct(s) => s.ident == name,
                syn::Item::Enum(e) => e.ident == name,
                _ => false,
            }),
            Foreign::Bits => items.iter().find_map(|item| match item {
                syn::Item::Macro(m) if m.mac.path.is_ident("bitflags") => {
                    bits_struct(&m.mac.tokens, name)
                }
                _ => None,
            }),
        }
        .ok_or_else(|| stale("not found"))?;
        let kind = if matches!(item, syn::Item::Enum(_)) {
            "enum"
        } else {
            "struct"
        };
        layouts.push(ForeignLayout {
            path: (*path).to_owned(),
            kind: kind.to_owned(),
            signature: crate::internals::shape_signature(&item)
                .expect("a struct or an enum")
                .to_token_stream()
                .to_string(),
        });
    }
    Ok(layouts)
}

/// The items of module `modules` (from the crate root) of the crate whose root is `lib`.
fn module_items(lib: &Path, modules: &[&str]) -> Result<Vec<syn::Item>, String> {
    let parse = |file: &Path| {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        syn::parse_file(&text)
            .map(|file| file.items)
            .map_err(|e| format!("{}: {e}", file.display()))
    };
    let mut items = parse(lib)?;
    let mut dir = lib.parent().expect("a source file has a folder").to_owned();
    for module in modules {
        let child = items
            .iter()
            .find_map(|item| match item {
                syn::Item::Mod(m) if m.ident == module => Some(m.content.clone()),
                _ => None,
            })
            .ok_or_else(|| format!("no module {module}"))?;
        items = match child {
            Some((_, inner)) => inner,
            None if dir.join(format!("{module}.rs")).is_file() => {
                parse(&dir.join(format!("{module}.rs")))?
            }
            None => parse(&dir.join(module).join("mod.rs"))?,
        };
        dir = dir.join(module);
    }
    Ok(items)
}

/// `struct name(Bits);` for the flags `name` in a `bitflags!` body, which declares them as
/// `struct Name: Bits { ... }`.
fn bits_struct(tokens: &proc_macro2::TokenStream, name: &str) -> Option<syn::Item> {
    use proc_macro2::TokenTree;
    let tokens: Vec<TokenTree> = tokens.clone().into_iter().collect();
    tokens.windows(4).find_map(|window| match window {
        [
            TokenTree::Ident(keyword),
            TokenTree::Ident(ident),
            TokenTree::Punct(colon),
            TokenTree::Ident(bits),
        ] if keyword == "struct" && ident == name && colon.as_char() == ':' => {
            syn::parse_str(&format!("struct {ident}({bits});")).ok()
        }
        _ => None,
    })
}

/// Crates whose types are the mod API's own: the staging pass never widens them.
const API_CRATES: &[&str] = &["mod_api", "mod_points", "horizon_engine_slots"];

/// The game's crate: its paths are spelled from `crate`.
const GAME: &str = "horizon_client";

/// A listed struct or enum.
pub struct Named {
    pub item: syn::Item,
    /// The module it is declared in, where its fields' types are written.
    pub module: String,
    pub lifetimes: usize,
    /// Generic over a type or a const: no one layout, so opaque.
    pub generic: bool,
    /// A layout the staging metadata does not prove can be accessed.
    pub opaque_reason: Option<&'static str>,
    /// Conditional fields or variants forbid copying and field access, but not handles.
    pub guarded: bool,
    /// Declared in another crate than the game's, where the table reads it.
    pub foreign: bool,
    /// Declared in a workspace crate below the game, whose fields the staged copy makes public
    /// (the mod API's own layouts and other crates' are not).
    pub widened: bool,
    /// `#[non_exhaustive]` on it or a case of it: another crate can't build it.
    pub sealed: bool,
}

/// A field of a listed type, in the numbering the table and the view share: declaration order,
/// across an enum's cases.
pub struct Field<'r> {
    /// The case it is in, for an enum: its index and name.
    pub case: Option<(u32, &'r syn::Ident)>,
    /// Its name, or its index in a tuple struct or case.
    pub member: syn::Member,
    pub ty: &'r Type,
}

/// A listed trait's declaration, and the module its signatures are written in.
pub struct TraitDecl {
    pub module: String,
    pub item: syn::ItemTrait,
}

/// The rule over one game's internals.
pub struct Rule {
    named: HashMap<String, Named>,
    symbols: std::collections::HashSet<String>,
    /// Import facts in each lexical module. Conflicting conditional imports stay unresolved.
    refs: HashMap<String, BTreeMap<String, Option<String>>>,
    memo: std::cell::RefCell<HashMap<String, Crossing>>,
    /// Every non-generic listed trait, by path: the trait objects that cross as method tables.
    traits: HashMap<String, TraitDecl>,
}

/// `a::b::C<'_>` as `a::b::C`.
pub fn bare(path: &str) -> String {
    path.split("::")
        .map(|segment| segment.split('<').next().unwrap_or(segment))
        .collect::<Vec<_>>()
        .join("::")
}

impl Rule {
    /// The rule over `internals`: every struct and enum they list, under no `cfg`.
    pub fn new(internals: &[Internal]) -> Self {
        let mut named = HashMap::new();
        let mut traits = HashMap::new();
        for internal in internals {
            if internal.cfg.is_empty()
                && internal.kind == "trait"
                && let Ok(syn::Item::Trait(item)) = syn::parse_str::<syn::Item>(&internal.signature)
                && item.generics.params.is_empty()
            {
                let path = bare(&internal.path);
                let module = path.rsplit_once("::").map_or("", |(module, _)| module);
                traits.insert(
                    path.clone(),
                    TraitDecl {
                        module: module.to_owned(),
                        item,
                    },
                );
            }
            if !internal.cfg.is_empty() || !matches!(internal.kind, "struct" | "enum") {
                continue;
            }
            let Ok(item) = syn::parse_str::<syn::Item>(&internal.signature) else {
                continue;
            };
            let generics = match &item {
                syn::Item::Struct(item) => &item.generics,
                syn::Item::Enum(item) => &item.generics,
                _ => continue,
            };
            let lifetimes = generics.lifetimes().count();
            let generic = generics.params.len() > lifetimes;
            let path = bare(&internal.path);
            let foreign = !path.starts_with(&format!("{GAME}::"));
            let open = |attrs: &[syn::Attribute]| {
                attrs.iter().any(|a| a.path().is_ident("non_exhaustive"))
            };
            let sealed = match &item {
                syn::Item::Struct(s) => open(&s.attrs),
                syn::Item::Enum(e) => open(&e.attrs) || e.variants.iter().any(|v| open(&v.attrs)),
                _ => false,
            };
            let conditional = |attrs: &[syn::Attribute]| {
                attrs
                    .iter()
                    .any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
            };
            let guarded = match &item {
                syn::Item::Struct(s) => s.fields.iter().any(|f| conditional(&f.attrs)),
                syn::Item::Enum(e) => e.variants.iter().any(|v| {
                    conditional(&v.attrs) || v.fields.iter().any(|f| conditional(&f.attrs))
                }),
                _ => false,
            };
            let opaque_reason = if let Some(reason) = internal.opaque_reason {
                Some(reason)
            } else if generic {
                Some("generic type")
            } else if foreign && API_CRATES.contains(&path.split("::").next().unwrap_or_default()) {
                // Listed lower types and reachable shapes are both widened in the staged copy;
                // the mod API's own layouts never are.
                Some("lower-crate layout not widened")
            } else {
                None
            };
            let (module, _) = path.rsplit_once("::").unwrap_or(("", &path));
            named.insert(
                path.clone(),
                Named {
                    module: module.to_owned(),
                    item,
                    lifetimes,
                    generic,
                    opaque_reason,
                    guarded,
                    foreign,
                    widened: foreign
                        && path.starts_with("horizon_")
                        && !API_CRATES.contains(&path.split("::").next().unwrap_or_default()),
                    sealed,
                },
            );
        }
        let mut refs: HashMap<String, BTreeMap<String, Option<String>>> = HashMap::new();
        for internal in internals {
            if !internal.cfg.is_empty()
                || !matches!(internal.kind, "struct" | "enum" | "type" | "fn" | "method")
            {
                continue;
            }
            let path = bare(&internal.path);
            let Some((owner, _)) = path.rsplit_once("::") else {
                continue;
            };
            let module = if internal.kind == "method" {
                let Some((module, _)) = owner.rsplit_once("::") else {
                    continue;
                };
                module
            } else {
                owner
            };
            let scope = refs.entry(module.to_owned()).or_default();
            for (written, resolved) in &internal.refs {
                scope
                    .entry(written.clone())
                    .and_modify(|known| {
                        if known.as_ref() != Some(resolved) {
                            *known = None;
                        }
                    })
                    .or_insert_with(|| Some(resolved.clone()));
            }
        }
        Self {
            named,
            symbols: internals
                .iter()
                .filter(|e| e.cfg.is_empty())
                .map(|e| bare(&e.path))
                .collect(),
            refs,
            memo: Default::default(),
            traits,
        }
    }

    /// The listed trait a `dyn` object of `ty`, written in `module`, is an object of, through
    /// references, parentheses and `Box`: `&dyn T`, `&mut dyn T`, `Box<dyn T>`, `&(dyn T + Send)`.
    pub fn trait_object(&self, module: &str, ty: &Type) -> Option<String> {
        match ty {
            Type::Reference(r) => self.trait_object(module, &r.elem),
            Type::Paren(p) => self.trait_object(module, &p.elem),
            Type::Group(g) => self.trait_object(module, &g.elem),
            Type::TraitObject(object) => object.bounds.iter().find_map(|bound| {
                let syn::TypeParamBound::Trait(bound) = bound else {
                    return None;
                };
                let found = self.resolve(module, &bound.path)?;
                self.traits.contains_key(&found).then_some(found)
            }),
            Type::Path(p) if p.qself.is_none() => {
                let last = p.path.segments.last()?;
                let boxed = last.ident == "Box"
                    && self.resolve(module, &p.path).is_none_or(|path| {
                        matches!(path.as_str(), "std::boxed::Box" | "alloc::boxed::Box")
                    });
                let PathArguments::AngleBracketed(args) = &last.arguments else {
                    return None;
                };
                match (boxed, args.args.first()) {
                    (true, Some(GenericArgument::Type(inner))) => self.trait_object(module, inner),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The listed type at `path`.
    pub fn named(&self, path: &str) -> Option<&Named> {
        self.named.get(path)
    }

    /// The fields of the listed type at `path`, numbered.
    pub fn fields<'r>(&'r self, path: &str) -> Vec<Field<'r>> {
        let fields = |case, fields: &'r syn::Fields| {
            fields.iter().enumerate().map(move |(i, field)| Field {
                case,
                member: field.ident.clone().map_or(
                    syn::Member::Unnamed(syn::Index::from(i)),
                    syn::Member::Named,
                ),
                ty: &field.ty,
            })
        };
        match self.named.get(path).map(|named| &named.item) {
            Some(syn::Item::Struct(item)) => fields(None, &item.fields).collect(),
            Some(syn::Item::Enum(item)) => item
                .variants
                .iter()
                .enumerate()
                .flat_map(|(i, v)| fields(Some((i as u32, &v.ident)), &v.fields))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Resolve import facts first, then explicit paths and declarations in the current module.
    /// A name that happens to be unique elsewhere is not proof of an import.
    pub(crate) fn resolve(&self, module: &str, written: &syn::Path) -> Option<String> {
        let spelled = written
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        if let Some(resolved) = self.refs.get(module).and_then(|scope| scope.get(&spelled)) {
            let resolved = resolved.as_ref()?;
            return Some(resolved.clone());
        }
        if matches!(
            written
                .segments
                .first()
                .map(|s| s.ident.to_string())
                .as_deref(),
            Some("std" | "core" | "alloc")
        ) {
            return Some(spelled);
        }
        let mut rest = spelled.as_str();
        let mut scope = module;
        let resolved = if let Some(rest) = rest.strip_prefix("crate::") {
            format!("{}::{rest}", module.split("::").next()?)
        } else if self.symbols.contains(rest) {
            rest.to_owned()
        } else {
            if let Some(tail) = rest.strip_prefix("self::") {
                rest = tail;
            }
            while let Some(tail) = rest.strip_prefix("super::") {
                scope = scope.rsplit_once("::")?.0;
                rest = tail;
            }
            format!("{scope}::{rest}")
        };
        self.symbols.contains(&resolved).then_some(resolved)
    }

    /// What `ty`, written in module `module` (where `Self` is `this`), is.
    pub fn shape(&self, module: &str, this: Option<&str>, ty: &Type) -> Shape {
        let opaque = Shape::Opaque(None);
        match ty {
            Type::Reference(r) => Shape::Ref {
                mutable: r.mutability.is_some(),
                to: Box::new(self.shape(module, this, &r.elem)),
            },
            Type::Paren(p) => self.shape(module, this, &p.elem),
            Type::Group(g) => self.shape(module, this, &g.elem),
            Type::Tuple(t) if t.elems.is_empty() => Shape::Unit,
            Type::Tuple(t) => Shape::Tuple(
                t.elems
                    .iter()
                    .map(|e| self.shape(module, this, e))
                    .collect(),
            ),
            Type::Array(a) => match &a.len {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(n),
                    ..
                }) => n.base10_parse().map_or(opaque, |n| {
                    Shape::Array(Box::new(self.shape(module, this, &a.elem)), n)
                }),
                _ => opaque,
            },
            Type::Slice(s) => Shape::Slice(Box::new(self.shape(module, this, &s.elem))),
            Type::Path(p) if p.qself.is_none() => self.path(module, this, &p.path),
            Type::TraitObject(object) => {
                // A listed trait's object keeps its identity: its table is that trait's methods.
                if let Some(path) = self.trait_object(module, ty) {
                    return Shape::Opaque(Some(format!("dyn {path}")));
                }
                let mut error = false;
                for bound in &object.bounds {
                    let syn::TypeParamBound::Trait(bound) = bound else {
                        continue;
                    };
                    let spelled = bound
                        .path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect::<Vec<_>>()
                        .join("::");
                    match self
                        .resolve(module, &bound.path)
                        .as_deref()
                        .unwrap_or(&spelled)
                    {
                        "std::error::Error" | "core::error::Error" => error = true,
                        "Send" | "Sync" | "std::marker::Send" | "std::marker::Sync" => {}
                        _ => return opaque,
                    }
                }
                if error { Shape::DynError } else { opaque }
            }
            _ => opaque,
        }
    }

    fn path(&self, module: &str, this: Option<&str>, path: &syn::Path) -> Shape {
        let opaque = Shape::Opaque(None);
        let Some(last) = path.segments.last() else {
            return opaque;
        };
        let name = last.ident.to_string();
        let args: Vec<Shape> = match &last.arguments {
            PathArguments::AngleBracketed(a) => a
                .args
                .iter()
                .filter_map(|a| match a {
                    GenericArgument::Type(t) => Some(self.shape(module, this, t)),
                    _ => None,
                })
                .collect(),
            PathArguments::None => Vec::new(),
            PathArguments::Parenthesized(_) => return opaque,
        };
        let first = path.segments[0].ident.to_string();
        if OPAQUE_CRATES.contains(&first.as_str())
            && foreign(&path.to_token_stream().to_string().replace(' ', "")).is_none()
        {
            return opaque;
        }
        if name == "Self" && path.segments.len() == 1 {
            return this.map_or(opaque, |this| self.named_shape(this));
        }
        let resolved = self.resolve(module, path);
        if let Some(found) = &resolved
            && self.named.contains_key(found)
        {
            return if args.is_empty() {
                self.named_shape(found)
            } else {
                opaque
            };
        }
        let spelled = path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        // These std and serde_json types have a Wire codec (`native_value`) but no leaf shape: they
        // are never walked. A foreign layout writes them without the import facts.
        let known = resolved.as_deref().unwrap_or(&spelled);
        let time = matches!(name.as_str(), "Instant" | "Duration")
            && match resolved.as_deref() {
                Some(path) => matches!(
                    path,
                    "std::time::Instant"
                        | "std::time::Duration"
                        | "web_time::Instant"
                        | "web_time::Duration"
                ),
                None => module.starts_with("winit"),
            };
        let opaque_codec = if time {
            Some(format!("std::time::{name}"))
        } else if matches!(known, "serde_json::Value") {
            Some("serde_json::Value".to_owned())
        } else if matches!(known, "std::path::Path" | "std::path::PathBuf") {
            Some("std::path::PathBuf".to_owned())
        } else {
            None
        };
        if let Some(codec) = opaque_codec
            && args.is_empty()
        {
            return Shape::Opaque(Some(codec));
        }
        if self
            .refs
            .get(module)
            .is_some_and(|scope| scope.get(&spelled) == Some(&None))
        {
            return opaque;
        }
        // Only prelude names can be inferred without import facts. Qualified and imported
        // names must identify the actual standard type, not just share its last segment.
        let prelude = path.segments.len() == 1
            && (SCALARS.iter().any(|(n, _)| *n == name)
                || matches!(name.as_str(), "Option" | "Box" | "Vec")
                || (name == "Result" && args.len() == 2));
        let builtin = resolved.as_deref().map_or(prelude, |p| {
            let Some((head, tail)) = p.split_once("::") else {
                return false;
            };
            if !matches!(head, "std" | "core" | "alloc") {
                return false;
            }
            match (head, tail) {
                (
                    "std" | "alloc",
                    "string::String"
                    | "boxed::Box"
                    | "sync::Arc"
                    | "rc::Rc"
                    | "borrow::Cow"
                    | "vec::Vec"
                    | "collections::VecDeque"
                    | "collections::vec_deque::VecDeque"
                    | "collections::BTreeMap"
                    | "collections::btree_map::BTreeMap"
                    | "collections::BTreeSet"
                    | "collections::btree_set::BTreeSet",
                ) => true,
                ("std" | "core", "option::Option" | "result::Result") => true,
                (
                    "std",
                    "collections::HashMap"
                    | "collections::hash_map::HashMap"
                    | "collections::HashSet"
                    | "collections::hash_set::HashSet",
                ) => true,
                ("std" | "core", primitive) => {
                    primitive.strip_prefix("primitive::").is_some_and(|n| {
                        SCALARS
                            .iter()
                            .any(|(s, scalar)| *s == n && *scalar != Scalar::String)
                    })
                }
                _ => false,
            }
        });
        // The game's `Result<T>` alias: `Result<T, Box<dyn Error>>`.
        if resolved
            .as_deref()
            .is_some_and(|p| ERROR_RESULTS.contains(&p))
        {
            return match <[Shape; 1]>::try_from(args) {
                Ok([ok]) => Shape::Result(
                    Box::new(ok),
                    Box::new(Shape::Wrapped(Wrapper::Box, Box::new(Shape::DynError))),
                ),
                Err(_) => opaque,
            };
        }
        if !builtin {
            return opaque;
        }
        let name = resolved
            .as_deref()
            .and_then(|p| p.rsplit("::").next())
            .unwrap_or(&name);
        let single = |args: Vec<Shape>| -> Option<Box<Shape>> {
            let [inner] = <[Shape; 1]>::try_from(args).ok()?;
            Some(Box::new(inner))
        };
        if let Some((_, scalar)) = SCALARS.iter().find(|(n, _)| *n == name) {
            return if args.is_empty() {
                Shape::Scalar(*scalar)
            } else {
                opaque
            };
        }
        if name == "Result" {
            return match <[Shape; 2]>::try_from(args) {
                Ok([ok, err]) => Shape::Result(Box::new(ok), Box::new(err)),
                Err(_) => opaque,
            };
        }
        if name == "Option" {
            return single(args).map_or(opaque, Shape::Option);
        }
        if let Some((_, wrapper)) = WRAPPERS.iter().find(|(n, _)| *n == name) {
            return single(args).map_or(opaque, |inner| Shape::Wrapped(*wrapper, inner));
        }
        if let Some((_, collection)) = COLLECTIONS.iter().find(|(n, _)| *n == name) {
            let keyed = matches!(collection, Collection::HashMap | Collection::BTreeMap);
            let wanted = if keyed { 2 } else { 1 };
            // A hasher argument is spelled as it is resolved, and never walked.
            return if args.len() < wanted {
                opaque
            } else {
                Shape::Collection(*collection, args)
            };
        }
        // A listed non-generic `Cell` must not capture `std::cell::Cell<T>`, for example.
        if !args.is_empty() {
            return opaque;
        }
        match self.resolve(module, path) {
            Some(found) => self.named_shape(&found),
            None => opaque,
        }
    }

    fn named_shape(&self, path: &str) -> Shape {
        match self.named.get(path) {
            Some(named) if named.opaque_reason.is_none() => {
                Shape::Named(path.to_owned(), named.lifetimes)
            }
            _ => Shape::Opaque(None),
        }
    }

    /// How `ty`, written in module `module`, crosses.
    pub fn crossing(&self, module: &str, ty: &Type) -> Crossing {
        self.of(&self.shape(module, None, ty), &mut Vec::new())
    }

    /// How `shape` crosses.
    pub fn crossing_of(&self, shape: &Shape) -> Crossing {
        self.of(shape, &mut Vec::new())
    }

    fn of(&self, shape: &Shape, open: &mut Vec<String>) -> Crossing {
        match shape {
            Shape::Scalar(Scalar::Wide(_)) => Crossing::Copy(2),
            Shape::Scalar(_) | Shape::Unit => Crossing::Copy(1),
            Shape::Ref { to, .. } | Shape::Wrapped(_, to) => self.of(to, open),
            Shape::Tuple(parts) => {
                if parts.iter().any(|p| self.of(p, open) == Crossing::Opaque) {
                    Crossing::Opaque
                } else {
                    self.all(parts.iter(), open)
                }
            }
            Shape::Array(elem, n) => match self.of(elem, open) {
                Crossing::Copy(leaves) if leaves * n <= LEAVES => Crossing::Copy(leaves * n),
                Crossing::Opaque => Crossing::Opaque,
                _ => Crossing::Handle,
            },
            Shape::Option(inner) => match self.of(inner, open) {
                Crossing::Copy(leaves) if leaves < LEAVES => Crossing::Copy(leaves + 1),
                Crossing::Opaque => Crossing::Opaque,
                _ => Crossing::Handle,
            },
            // Nodes erase pointers to `*mut ()`; a slice's metadata cannot survive that.
            Shape::Slice(_) => Crossing::Opaque,
            // A tagged case holds either payload, so the whole is walked, never copied; an
            // unprojectable payload is a leaf, not a reason to hide the other.
            Shape::Result(ok, err) => {
                if self.of(ok, open) == Crossing::Opaque && self.of(err, open) == Crossing::Opaque {
                    Crossing::Opaque
                } else {
                    Crossing::Handle
                }
            }
            Shape::DynError => Crossing::Copy(1),
            Shape::Collection(collection, args) => {
                // Sets have no stable addressable element nodes, even though their owned
                // value shape can be used by a native codec.
                if matches!(collection, Collection::HashSet | Collection::BTreeSet) {
                    return Crossing::Opaque;
                }
                let walked = match collection {
                    Collection::HashMap | Collection::BTreeMap => &args[..2],
                    _ => &args[..1],
                };
                if args.len() != walked.len()
                    || walked.iter().any(|a| self.of(a, open) == Crossing::Opaque)
                {
                    Crossing::Opaque
                } else {
                    Crossing::Handle
                }
            }
            Shape::Named(path, _) => self.named_crossing(path, open),
            Shape::Opaque(_) => Crossing::Opaque,
        }
    }

    /// How the listed type at `path` crosses, each once.
    fn named_crossing(&self, path: &str, open: &mut Vec<String>) -> Crossing {
        if let Some(known) = self.memo.borrow().get(path) {
            return *known;
        }
        // A type that holds itself, through a box or a list, is walked, not copied.
        if open.iter().any(|p| p == path) {
            return Crossing::Handle;
        }
        let named = &self.named[path];
        if named.opaque_reason.is_some() {
            return Crossing::Opaque;
        }
        open.push(path.to_owned());
        let parts: Vec<Shape> = self
            .fields(path)
            .iter()
            .map(|field| self.shape(&named.module, Some(path), field.ty))
            .collect();
        let crossing = match self.all(parts.iter(), open) {
            Crossing::Copy(_) if named.guarded => Crossing::Opaque,
            crossing => crossing,
        };
        open.pop();
        self.memo.borrow_mut().insert(path.to_owned(), crossing);
        crossing
    }

    /// A type made of `parts`: a copy when each is and their leaves fit, else a handle, whose parts
    /// that can't cross simply can't be reached. A type with no parts is one leaf (`()`, a unit
    /// case).
    fn all<'s>(&self, parts: impl Iterator<Item = &'s Shape>, open: &mut Vec<String>) -> Crossing {
        let mut leaves = 0;
        let mut copy = true;
        let mut any = false;
        for part in parts {
            any = true;
            match self.of(part, open) {
                Crossing::Copy(n) => leaves += n,
                Crossing::Handle | Crossing::Opaque => copy = false,
            }
        }
        match (any, copy) {
            (false, _) => Crossing::Copy(1),
            (true, true) if leaves <= LEAVES => Crossing::Copy(leaves),
            _ => Crossing::Handle,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn internal(path: &str, kind: &'static str, signature: &str) -> Internal {
        Internal {
            path: path.into(),
            kind,
            cfg: String::new(),
            signature: signature.into(),
            hash: String::new(),
            shape_only: false,
            opaque_reason: None,
            refs: BTreeMap::new(),
        }
    }

    #[test]
    fn guarded_copy_layouts_stay_opaque() {
        for signature in [
            "struct Small { #[cfg(feature = \"x\")] value: u8 }",
            "enum Small { Always, #[cfg(feature = \"x\")] Conditional(u8) }",
        ] {
            let kind = if signature.starts_with("enum") {
                "enum"
            } else {
                "struct"
            };
            let rule = Rule::new(&[internal("horizon_client::Small", kind, signature)]);
            let shape = rule.shape("horizon_client", None, &syn::parse_str("Small").unwrap());
            assert!(matches!(shape, Shape::Named(..)));
            assert_eq!(rule.crossing_of(&shape), Crossing::Opaque);
        }
    }

    fn rule() -> Rule {
        Rule::new(&[
            internal(
                "horizon_client::Pos",
                "struct",
                "struct Pos { x : i32 , y : i32 , z : i32 , }",
            ),
            internal(
                "horizon_client::Mode",
                "enum",
                "enum Mode { Survival , Creative , Custom (u8) , }",
            ),
            internal(
                "horizon_client::Player",
                "struct",
                "struct Player { pos : Pos , mode : Mode , name : String , }",
            ),
            internal(
                "horizon_client::World",
                "struct",
                "struct World { players : Vec < Player > , spawn : Pos , }",
            ),
            internal(
                "horizon_client::Game",
                "struct",
                "struct Game < 'a > { world : World , ui : RefCell < u8 > , }",
            ),
            internal(
                "horizon_client::Big",
                "struct",
                "struct Big { a : [u64 ; 17] , }",
            ),
            internal(
                "horizon_client::Tree",
                "struct",
                "struct Tree { kids : Vec < Tree > , }",
            ),
            internal(
                "horizon_client::Cache",
                "struct",
                "struct Cache < T > { item : T , }",
            ),
            internal(
                "horizon_client::a::Pos",
                "struct",
                "struct Pos { only : bool , }",
            ),
        ])
    }

    fn of(rule: &Rule, module: &str, ty: &str) -> Crossing {
        rule.crossing(module, &syn::parse_str(ty).unwrap())
    }

    #[test]
    fn result_is_a_two_case_enum_with_a_display_text_for_dyn_error() {
        let mut thing = internal("horizon_client::game::Thing", "struct", "struct Thing;");
        thing
            .refs
            .insert("Result".into(), "horizon_client_core::Result".into());
        let rule = Rule::new(&[thing]);
        let shape =
            |ty: &str| rule.shape("horizon_client::game", None, &syn::parse_str(ty).unwrap());
        let boxed = Shape::Wrapped(Wrapper::Box, Box::new(Shape::DynError));
        let unit = Shape::Unit;
        // The game's alias fixes the error to `Box<dyn Error>`.
        assert_eq!(
            shape("Result<bool>"),
            Shape::Result(
                Box::new(Shape::Scalar(Scalar::Bool)),
                Box::new(boxed.clone())
            )
        );
        assert_eq!(
            shape("std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>"),
            Shape::Result(Box::new(unit), Box::new(boxed))
        );
        // Written in full, `Result` is the std type.
        let rule_plain = Rule::new(&[]);
        let plain = |ty: &str| rule_plain.shape("m", None, &syn::parse_str(ty).unwrap());
        assert_eq!(
            plain("Result<u8, String>"),
            Shape::Result(
                Box::new(Shape::Scalar(Scalar::Unsigned("u8"))),
                Box::new(Shape::Scalar(Scalar::String))
            )
        );
        // An Err type that does not cross stays a leaf: the Result is still walked.
        assert!(matches!(
            plain("Result<u8, wgpu::Error>"),
            Shape::Result(_, err) if matches!(*err, Shape::Opaque(_))
        ));
        assert_eq!(
            rule_plain.crossing_of(&plain("Result<u8, wgpu::Error>")),
            Crossing::Handle
        );
        assert_eq!(
            rule_plain.crossing_of(&plain("Result<wgpu::A, wgpu::B>")),
            Crossing::Opaque
        );
        // A one-argument `Result` that is not the game's alias is some other type.
        assert!(matches!(
            Rule::new(&[]).shape("m", None, &syn::parse_str("Result<u8>").unwrap()),
            Shape::Opaque(_)
        ));
    }

    #[test]
    fn plain_data_copies_up_to_its_leaves_and_the_rest_is_walked_or_opaque() {
        let rule = rule();
        assert_eq!(of(&rule, "horizon_client", "[i32; 3]"), Crossing::Copy(3));
        assert_eq!(of(&rule, "horizon_client", "Pos"), Crossing::Copy(3));
        assert_eq!(
            of(&rule, "horizon_client", "Option<Pos>"),
            Crossing::Copy(4)
        );
        assert_eq!(of(&rule, "horizon_client", "(u8, &str)"), Crossing::Copy(2));
        assert_eq!(of(&rule, "horizon_client", "Mode"), Crossing::Copy(1));
        // A player is plain data: a copy, its name one leaf.
        assert_eq!(of(&rule, "horizon_client", "Player"), Crossing::Copy(5));
        assert_eq!(of(&rule, "horizon_client", "World"), Crossing::Handle);
        assert_eq!(of(&rule, "horizon_client", "Big"), Crossing::Handle);
        assert_eq!(of(&rule, "horizon_client", "Tree"), Crossing::Handle);
        assert_eq!(
            of(&rule, "horizon_client", "&mut Vec<Pos>"),
            Crossing::Handle
        );
        // A cell in it makes `Game` no copy, but a handle: its other fields are reached one by
        // one, and the cell can't be.
        assert_eq!(of(&rule, "horizon_client", "Game<'_>"), Crossing::Handle);
        assert_eq!(
            of(&rule, "horizon_client", "(u8, RefCell<u8>)"),
            Crossing::Opaque
        );
        assert_eq!(of(&rule, "horizon_client", "Cache<u8>"), Crossing::Opaque);
        for opaque in [
            "Box<dyn Fn()>",
            "impl Iterator<Item = u8>",
            "*const u8",
            "wgpu::Device",
            "Mutex<u8>",
            "Unlisted",
        ] {
            assert_eq!(
                of(&rule, "horizon_client", opaque),
                Crossing::Opaque,
                "{opaque}"
            );
        }
    }

    #[test]
    fn sets_are_opaque() {
        let rule = rule();
        for ty in [
            "std::collections::HashSet<u32>",
            "std::collections::BTreeSet<u32>",
        ] {
            assert_eq!(of(&rule, "horizon_client", ty), Crossing::Opaque);
        }
    }

    #[test]
    fn custom_string_and_vec_are_not_standard_builtins() {
        let mut owner = internal(
            "horizon_client::Owner",
            "struct",
            "struct Owner { value: String }",
        );
        owner
            .refs
            .insert("String".into(), "horizon_client::custom::String".into());
        owner
            .refs
            .insert("Vec".into(), "horizon_client::custom::Vec".into());
        let rule = Rule::new(&[
            owner,
            internal(
                "horizon_client::custom::String",
                "struct",
                "struct String { x: u8, y: bool }",
            ),
            internal(
                "horizon_client::custom::Vec",
                "struct",
                "struct Vec<T> { value: T }",
            ),
        ]);
        assert_eq!(
            rule.shape("horizon_client", None, &syn::parse_str("String").unwrap()),
            Shape::Named("horizon_client::custom::String".into(), 0)
        );
        assert_eq!(of(&rule, "horizon_client", "String"), Crossing::Copy(2));
        assert_eq!(of(&rule, "horizon_client", "Vec<u8>"), Crossing::Opaque);
        assert_eq!(
            of(&rule, "horizon_client::custom", "String"),
            Crossing::Copy(2)
        );
        assert_eq!(
            of(&rule, "horizon_client::custom", "Vec<u8>"),
            Crossing::Opaque
        );
        assert_eq!(
            of(&rule, "horizon_client", "std::string::String"),
            Crossing::Copy(1)
        );
    }

    fn checkout() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    #[ignore = "daily"]
    fn checked_in_foreign_layouts_match_the_locked_source() {
        let root = checkout();
        assert_eq!(
            locked_foreign_layouts(&root).unwrap(),
            std::fs::read_to_string(root.join(CROSSING_FOREIGN_FILE)).unwrap(),
            "{CROSSING_FOREIGN_FILE} is stale: run `cargo run -p xtask -- mod-api`"
        );
    }

    #[test]
    fn listed_winit_types_cross_by_copy_and_the_rest_of_winit_stays_opaque() {
        let mut internals = foreign_internals();
        assert_eq!(internals.len(), CROSSING_FOREIGN.len());
        internals.push(internal(
            "horizon_client::Held",
            "struct",
            "struct Held { key : winit :: keyboard :: PhysicalKey , mods : winit :: keyboard :: ModifiersState , }",
        ));
        let rule = Rule::new(&internals);
        for (path, leaves) in [
            ("winit::keyboard::KeyCode", 1),
            // A case's fields all count: `Code(KeyCode)` and the four native scancodes.
            ("winit::keyboard::PhysicalKey", 5),
            ("winit::keyboard::NativeKeyCode", 4),
            ("winit::keyboard::KeyLocation", 1),
            ("winit::keyboard::ModifiersState", 1),
            ("winit::event::ElementState", 1),
            ("winit::event::MouseButton", 1),
            ("horizon_client::Held", 6),
        ] {
            assert_eq!(
                of(&rule, "horizon_client", path),
                Crossing::Copy(leaves),
                "{path}"
            );
            // Only winit's `#[non_exhaustive]` `KeyCode` is sealed: another crate never builds
            // it, so it and what holds it are read-only.
            assert_eq!(
                rule.named(path).unwrap().sealed,
                path == "winit::keyboard::KeyCode",
                "{path}"
            );
        }
        assert_eq!(
            rule.fields("winit::keyboard::ModifiersState")[0]
                .ty
                .to_token_stream()
                .to_string(),
            "u32"
        );
        for opaque in [
            "winit::window::Window",
            "winit::keyboard::Key",
            "winit::event::KeyEvent",
        ] {
            assert_eq!(
                of(&rule, "horizon_client", opaque),
                Crossing::Opaque,
                "{opaque}"
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_stale_crossing_foreign_path_fails_with_its_name() {
        let metadata = cargo_metadata(&checkout()).unwrap();
        for (stale, foreign) in [
            ("winit::keyboard::Gone", Foreign::Layout),
            ("winit::gone::KeyCode", Foreign::Layout),
            ("winit::keyboard::KeyCode", Foreign::Bits),
        ] {
            let error = read_foreign(&[(stale, foreign)], |krate| locked_lib(&metadata, krate))
                .unwrap_err();
            assert!(error.starts_with(&format!("{stale}: ")), "{error}");
        }
    }

    #[test]
    fn a_name_resolves_in_its_own_module_first() {
        let rule = rule();
        assert_eq!(of(&rule, "horizon_client::a", "Pos"), Crossing::Copy(1));
        assert_eq!(of(&rule, "horizon_client", "Pos"), Crossing::Copy(3));
        assert_eq!(
            of(&rule, "horizon_client::b", "super::a::Pos"),
            Crossing::Copy(1)
        );
        assert_eq!(of(&rule, "horizon_client::b", "a::Pos"), Crossing::Opaque);
        assert_eq!(of(&rule, "horizon_client::b", "Player"), Crossing::Opaque);
        assert_eq!(of(&rule, "horizon_client", "Pos<u8>"), Crossing::Opaque);
    }
}
