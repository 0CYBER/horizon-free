//! Injection points (arismoko/horizon#589): every function of the game module that [`point_shape::eligible`]
//! admits and the API can carry is a point, and so is each such function marked `#[horizon_mod_api_macros::replaceable]` in
//! the crates below it. The staging pass ([`stage`]) puts `#[point]` on each in a staged copy of the
//! game's source, the copy the prebuilt game builds compile;
//! the generator declares records, copy-in constructors and typed dispatch in
//! `horizon_engine_slots`, and mod-facing registration and guest exports in `mod_points`.
//! `points.json` is the compiled slot catalog. These are build outputs, never committed.

use crate::replace::marked;
use crate::source::{crate_files, crate_root, game_files, offset, owner, plain_path, unraw};
use mod_source::modules::gated;
use quote::{ToTokens, quote};
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{ImplItem, Item, Type};

/// One point, as `points.json` lists it.
#[derive(Debug, serde::Serialize)]
pub struct GamePoint {
    /// Why its API listeners are refused at registration, native and wasm alike: the first
    /// parameter with no API type, naming it. Absent when the point has an API record.
    pub api_refusal: Option<String>,
    pub name: String,
    /// The function, as an injection's `target` names it.
    pub target: String,
    /// The side of a `Game` method's point, which picks its API listeners' host; `none` off `Game`.
    pub side: &'static str,
    /// The API listener's host: `host` (`&self`), the side's writing host (`&mut self`), or `none`.
    pub host: &'static str,
    /// The record its API listeners get, in `mod_points`.
    pub record: String,
    /// The parameters passed to API listeners, with their types, in order.
    pub args: Vec<(String, String)>,
    /// What its listeners may do with the return: `cancel`, `result` (an API-typed value they
    /// may set at the head or change at the return) or `none`.
    pub returns: &'static str,
    /// The function's return type as written (`()` when it has none). A `cancel` point that
    /// returns `Result<(), E>` has no `replace` listener, which would have to produce it.
    pub return_ty: String,
    /// Whether its API listeners run at the return: not when the value returned borrows.
    pub at_return: bool,
    /// [`mod_surface::contract_hash`] of everything above but the name: equal hashes on
    /// two builds, the same point for a mod's `[[inject]]` row.
    pub signature_hash: String,
    #[serde(skip)]
    record_item: String,
    /// Its typed function constructors, place ids and dispatch functions
    /// ([`point_shape::Shape::api_items`]).
    #[serde(skip)]
    api: [String; 3],
}

impl GamePoint {
    /// The return type a `replace` at this point's head can't produce: none when it returns `()`
    /// (the cancel is forced) or an API value (the listener's). The rule `horizon-mod-point-emit`
    /// applies when a mod builds, kept for the loader of a module built elsewhere.
    fn replace_refusal(&self) -> Option<&str> {
        match self.returns {
            "result" => None,
            "cancel" if self.return_ty == "()" => None,
            _ => Some(&self.return_ty),
        }
    }
}

impl GamePoint {
    /// The point of `found`'s function, whose shape is `shape`, as `points.json` lists it.
    pub(crate) fn new(shape: point_shape::Shape, found: &Found<'_>, target: String) -> Self {
        let api = shape.api_items(&target);
        GamePoint {
            signature_hash: String::new(),
            record_item: if shape.api() {
                shape.record_item(&target).to_string()
            } else {
                String::new()
            },
            api: [
                api.listeners.to_string(),
                api.variants.to_string(),
                api.dispatch.to_string(),
            ],
            at_return: shape.api() && shape.has_return(),
            api_refusal: shape.api_refusal.clone(),
            target,
            side: shape.side.map_or("none", point_shape::Side::name),
            host: match (shape.receiver, shape.side) {
                (_, None) => "none",
                (point_shape::Receiver::Mut, Some(side)) => side.name(),
                (_, Some(_)) => "host",
            },
            args: shape
                .args
                .iter()
                .map(|arg| {
                    (
                        arg.name.to_string(),
                        arg.ty.to_token_stream().to_string().replace("& ", "&"),
                    )
                })
                .collect(),
            record: shape.record().to_string(),
            returns: match (&shape.returns, shape.api()) {
                (_, false) => "none",
                (point_shape::Returns::Unit | point_shape::Returns::UnitResult, true) => "cancel",
                (point_shape::Returns::Value(_), true) => "result",
                (point_shape::Returns::Opaque, true) => "none",
            },
            return_ty: match &found.sig.output {
                syn::ReturnType::Default => "()".to_owned(),
                syn::ReturnType::Type(_, ty) => point_shape::spelling(ty),
            },
            name: shape.name,
        }
        .hashed()
    }

    /// The point with its [`GamePoint::signature_hash`] filled in.
    fn hashed(mut self) -> Self {
        self.signature_hash = mod_surface::contract_hash(&(
            &self.target,
            self.side,
            self.host,
            &self.record,
            &self.args,
            self.returns,
            &self.return_ty,
            self.at_return,
            &self.api_refusal,
        ));
        self
    }
}

/// The engine slots and their dependencies, by library name. Their functions cannot
/// be points: dispatch would have to reach back up to the slots that own it.
pub(crate) fn below_points(root: &Path) -> Result<std::collections::BTreeSet<String>, String> {
    let mut seen = std::collections::BTreeSet::from(["horizon_engine_slots".to_owned()]);
    let mut pending = vec![root.join("crates/horizon-engine-slots")];
    while let Some(package) = pending.pop() {
        let manifest = package.join("Cargo.toml");
        let text =
            fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        let manifest: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
        // Build dependencies too: engine slots are generated by `horizon-mod-api-gen`.
        for path in ["dependencies", "build-dependencies"]
            .into_iter()
            .filter_map(|kind| manifest.get(kind).and_then(toml::Value::as_table))
            .flatten()
            .filter_map(|(_, dep)| dep.get("path").and_then(toml::Value::as_str))
        {
            let dependency = package.join(path);
            let name = dependency
                .file_name()
                .map(|name| name.to_string_lossy().replace('-', "_"))
                .unwrap_or_default();
            if seen.insert(name) {
                pending.push(dependency);
            }
        }
    }
    Ok(seen)
}

/// The game module's files, slot lists and marked crates' files, for a build script to track.
pub fn tracked(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<_> = game_files(&root.join("crates/horizon-client/src"), true)?
        .into_iter()
        .map(|file| file.path)
        .collect();
    let crates: std::collections::BTreeSet<_> = marked(root)?
        .into_iter()
        .filter_map(|(_, path)| path.split("::").next().map(str::to_owned))
        .collect();
    for krate in crates {
        files.extend(crate_files(root, &krate)?.into_iter().map(|file| file.path));
    }
    Ok(files)
}

/// A point's function, as found in a parsed file.
#[derive(Clone)]
pub(crate) struct Found<'a> {
    pub(crate) module: String,
    attrs: &'a [syn::Attribute],
    vis: &'a syn::Visibility,
    pub(crate) sig: &'a syn::Signature,
    /// The `impl`'s type and generics; none for a free function.
    owner: Option<(&'a Type, &'a syn::Generics)>,
    /// The types its signature names that cross as `mod_api` types, as written, with the API
    /// type's path and whether it converts (an engine type's [`MIRRORS`] entry).
    mirrors: Vec<(String, String, bool)>,
    /// The replacement rule that leaves it without a cell, and so without a point site.
    unreplaceable: Option<&'static str>,
}

impl Found<'_> {
    /// A method of `Game`, taking `self`: its point keeps the method's name and gets the game as
    /// its host. `Game`'s associated functions without `self` are named and hosted like any other
    /// type's.
    fn on_game(&self) -> bool {
        matches!(self.sig.inputs.first(), Some(syn::FnArg::Receiver(_)))
            && self.owner.is_some_and(|(ty, _)| matches!(ty, Type::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 && path.path.segments[0].ident == "Game"))
    }

    /// Its path in its crate, as an injection's `target` names it.
    pub(crate) fn target(&self) -> String {
        match self.owner {
            Some((ty, _)) => format!("{}::{}::{}", self.module, owner(ty), self.sig.ident),
            None => format!("{}::{}", self.module, self.sig.ident),
        }
    }

    /// Its point's name: a `Game` method's own name; else its path from the game module (`game_…`),
    /// or in a lower crate from its crate's name less `horizon_`, snake-cased. The form is not
    /// injective (`a::b_c` and `a_b::c` collide): `all_points` refuses two points of one name.
    fn name(&self) -> String {
        if self.on_game() {
            return unraw(&self.sig.ident);
        }
        let module = match self.module.strip_prefix("horizon_client::") {
            Some(rest) => rest.to_owned(),
            None => self
                .module
                .strip_prefix("horizon_")
                .unwrap_or(&self.module)
                .to_owned(),
        };
        let mut parts: Vec<String> = module
            .split("::")
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect();
        if let Some((Type::Path(path), _)) = self.owner
            && let Some(last) = path.path.segments.last()
        {
            parts.push(snake(&unraw(&last.ident)));
        }
        parts.push(unraw(&self.sig.ident));
        parts.join("_")
    }

    /// The attribute the pass writes on it.
    fn attribute(&self) -> String {
        let mut parts = Vec::new();
        if self.on_game() {
            parts.push("side = \"both\"".to_owned());
        } else {
            parts.push(format!("name = \"{}\"", self.name()));
        }
        if self.owner.is_none() {
            parts.push("free".into());
        }
        for (word, convert) in [("api", false), ("mirror", true)] {
            let named: Vec<_> = self
                .mirrors
                .iter()
                .filter(|(_, _, converts)| *converts == convert)
                .map(|(written, api, _)| format!("{written} = {api:?}"))
                .collect();
            if !named.is_empty() {
                parts.push(format!("{word}({})", named.join(", ")));
            }
        }
        parts.join(", ")
    }

    /// Its shape. A function without a replacement cell has no site a listener could run at:
    /// its API listeners are refused, naming the rule.
    fn shape(&self) -> Result<point_shape::Shape, String> {
        let mut shape =
            point_shape::shape(self.attribute().parse().expect("the attribute"), self.sig)
                .map_err(|e| format!("{}: {e}", self.target()))?;
        if let Some(rule) = self.unreplaceable {
            shape.api_refusal = Some(format!(
                "the function has no point site: it takes no replacement ({rule})"
            ));
        }
        Ok(shape)
    }
}

/// `MapData` as `map_data`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The point functions of one parsed file, with their module paths: in the game module, every
/// eligible function of its inherent `impl`s (a `Game` method only with a `&self`/`&mut self`
/// receiver) and its free functions; elsewhere, those `keep` admits. Inline modules every build
/// compiles count too.
pub(crate) fn file_points<'a>(
    module: &str,
    items: &'a [Item],
    keep: &dyn Fn(&Found<'_>) -> bool,
    out: &mut Vec<Found<'a>>,
) {
    file_functions(module, items, keep, &point_shape::eligible, out);
}

pub(crate) fn file_functions<'a>(
    module: &str,
    items: &'a [Item],
    keep: &dyn Fn(&Found<'_>) -> bool,
    eligible: &dyn Fn(&[syn::Attribute], &syn::Signature) -> bool,
    out: &mut Vec<Found<'a>>,
) {
    for item in items {
        match item {
            Item::Mod(inline) if !gated(&inline.attrs) => {
                if let Some((_, inner)) = &inline.content {
                    file_functions(
                        &format!("{module}::{}", inline.ident),
                        inner,
                        keep,
                        eligible,
                        out,
                    );
                }
            }
            Item::Fn(function)
                if !gated(&function.attrs) && eligible(&function.attrs, &function.sig) =>
            {
                let found = Found {
                    module: module.to_owned(),
                    attrs: &function.attrs,
                    vis: &function.vis,
                    sig: &function.sig,
                    owner: None,
                    mirrors: Vec::new(),
                    unreplaceable: None,
                };
                if keep(&found) {
                    out.push(found);
                }
            }
            Item::Impl(block) if block.trait_.is_none() && !gated(&block.attrs) => {
                let Type::Path(ty) = &*block.self_ty else {
                    continue;
                };
                if ty.qself.is_some() {
                    continue;
                }
                for member in &block.items {
                    if let ImplItem::Fn(method) = member
                        && eligible(&method.attrs, &method.sig)
                    {
                        let found = Found {
                            module: module.to_owned(),
                            attrs: &method.attrs,
                            vis: &method.vis,
                            sig: &method.sig,
                            owner: Some((&block.self_ty, &block.generics)),
                            mirrors: Vec::new(),
                            unreplaceable: None,
                        };
                        if keep(&found) {
                            out.push(found);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// A parsed source file of the game's crates, with its points.
struct Scanned {
    path: PathBuf,
    module: String,
    source: String,
    parsed: syn::File,
    /// The crate's package, when it is a lower crate: the pass gives it the points' dependencies.
    package: Option<PathBuf>,
}

/// The `mod_api` types a record may hold ([`crate::build::wire_types`]), by path.
pub type WireTypes = std::collections::BTreeSet<String>;

/// A file to read for points, with its crate's package when it is a lower crate's.
type Input = (mod_source::modules::ModuleFile, Option<PathBuf>);

/// Every file with points: the game module's, then each marked lower crate's that is not built
/// under the points' own crate.
fn scan(root: &Path) -> Result<(Vec<Scanned>, Vec<(String, String)>), String> {
    let (files, list) = inputs(root)?;
    let mut scanned = Vec::new();
    for (file, package) in files {
        let source = fs::read_to_string(&file.path).map_err(|e| e.to_string())?;
        let parsed =
            syn::parse_file(&source).map_err(|e| format!("{}: {e}", file.path.display()))?;
        scanned.push(Scanned {
            path: file.path,
            module: file.module,
            source,
            parsed,
            package,
        });
    }
    Ok((scanned, list))
}

/// The files [`scan`] reads, and the marked functions of the crates below the game.
fn inputs(root: &Path) -> Result<(Vec<Input>, Vec<(String, String)>), String> {
    let mut files = Vec::new();
    for file in game_files(&root.join("crates/horizon-client/src"), true)? {
        files.push((file, None));
    }
    // The wasm host's listener tests listen at points of their own fixture file.
    #[cfg(feature = "listener-fixture")]
    {
        let path = root.join("crates/horizon-mod-wasm/tests/fixture/src/points.rs");
        if path.is_file() {
            files.push((
                mod_source::modules::ModuleFile {
                    module: "listener_fixture".into(),
                    path,
                    cfg: Vec::new(),
                },
                None,
            ));
        }
    }
    let list = marked(root)?;
    let below = below_points(root)?;
    let mut crates = std::collections::BTreeSet::new();
    for (_, path) in &list {
        let krate = path.split("::").next().unwrap_or_default();
        // A crate the points' own crate is built on has replaceable functions and no points:
        // its slots would have to reach up to the table.
        if !below.contains(krate) {
            crates.insert(krate.to_owned());
        }
    }
    let mut workspace_manifest = None;
    for krate in &crates {
        let (package, _) = crate_root(root, krate)?;
        let manifest = crate::replace::manifest(&package)?;
        let dependency_tables = manifest.get("dependencies").into_iter().chain(
            manifest
                .get("target")
                .and_then(toml::Value::as_table)
                .into_iter()
                .flat_map(|targets| targets.values())
                .filter_map(|target| target.get("dependencies")),
        );
        let mut has_slots = false;
        for (key, dependency) in dependency_tables
            .filter_map(toml::Value::as_table)
            .flatten()
        {
            let dependency =
                if dependency.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                    let workspace = workspace_manifest
                        .get_or_insert_with(|| crate::replace::manifest(root))
                        .as_ref()
                        .map_err(Clone::clone)?;
                    workspace
                        .get("workspace")
                        .and_then(|workspace| workspace.get("dependencies"))
                        .and_then(|dependencies| dependencies.get(key.as_str()))
                        .unwrap_or(dependency)
                } else {
                    dependency
                };
            let name = dependency
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(key);
            if name == "horizon-engine-slots" {
                has_slots = true;
                break;
            }
        }
        if !has_slots {
            return Err(format!(
                "crate {krate} has points but does not depend on horizon-engine-slots"
            ));
        }
        for file in crate_files(root, krate)? {
            files.push((file, Some(package.clone())));
        }
    }
    Ok((files, list))
}

impl Scanned {
    /// This file's points: all of a game-module file's, or the marked ones of a lower crate's.
    fn points<'a>(&'a self, list: &[(String, String)], wire: &WireTypes) -> Vec<Found<'a>> {
        let mut out = Vec::new();
        let keep = |found: &Found<'_>| {
            // A staged copy also holds what the replacement pass wrote beside each function.
            if found.sig.ident.to_string().starts_with("__fnr_") {
                return false;
            }
            if self.package.is_none() {
                return true;
            }
            let target = plain_path(&found.target());
            list.iter().any(|(_, path)| *path == target)
        };
        file_points(&self.module, &self.parsed.items, &keep, &mut out);
        let uses = imports(&self.parsed.items, "horizon_client");
        // A point is installed through its function's replacement cell: only a replaceable
        // function has one.
        let none = std::collections::BTreeSet::new();
        let replaceable = crate::replace::targets(
            &self.module,
            &self.parsed,
            &none,
            &crate::replace::OwnerTypes::default(),
        );
        for found in &mut out {
            // Only the game's own crate has the conversions a mirror goes through.
            found.mirrors = mirrors(found.sig, &uses, wire, self.package.is_none());
            if !replaceable.contains(&found.target()) {
                found.unreplaceable = Some(
                    crate::replace::signature_exclusion(found.attrs, found.sig).unwrap_or(
                        match found.owner {
                            Some((_, generics)) if !crate::replace::lifetimes_only(generics) => {
                                "impl type/const generics"
                            }
                            _ => "no replacement cell",
                        },
                    ),
                );
            }
        }
        out
    }
}

/// The engine types a point carries as their `mod_api` mirrors, by full path, and the mirror. The
/// game converts each through its `TryIntoApi` and `TryIntoEngine` (`game::points`).
pub const MIRRORS: &[(&str, &str)] = &[
    ("horizon_client::text::Text", "::mod_api::Text"),
    ("horizon_client_core::text::Text", "::mod_api::Text"),
    (
        "horizon_world::registry::BlockStateId",
        "::mod_api::BlockState",
    ),
];

/// A file's top-level `use`s: each name it brings in, with the path it names (`crate::` as
/// `krate`). Globs and `self`/`super` paths are left out: a type they name has no mirror.
pub(crate) fn imports(items: &[Item], krate: &str) -> Vec<(String, String)> {
    fn walk(prefix: &str, tree: &syn::UseTree, out: &mut Vec<(String, String)>) {
        let join = |name: &str| {
            if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}::{name}")
            }
        };
        match tree {
            syn::UseTree::Path(path) => walk(&join(&path.ident.to_string()), &path.tree, out),
            syn::UseTree::Name(name) => {
                out.push((name.ident.to_string(), join(&name.ident.to_string())))
            }
            syn::UseTree::Rename(rename) => {
                out.push((rename.rename.to_string(), join(&rename.ident.to_string())))
            }
            syn::UseTree::Group(group) => {
                for tree in &group.items {
                    walk(prefix, tree, out);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
    let mut out = Vec::new();
    for item in items {
        if let Item::Use(item) = item {
            walk("", &item.tree, &mut out);
        }
    }
    for (_, path) in &mut out {
        if let Some(rest) = path.strip_prefix("crate::") {
            *path = format!("{krate}::{rest}");
        }
    }
    out
}

/// The leaves of `sig` that name a `mod_api` type in `wire`, or (with `convert`) a [`MIRRORS`]
/// type, through the file's `imports` or in full.
fn mirrors(
    sig: &syn::Signature,
    imports: &[(String, String)],
    wire: &WireTypes,
    convert: bool,
) -> Vec<(String, String, bool)> {
    let mut found: Vec<(String, String, bool)> = Vec::new();
    for leaf in point_shape::leaves(sig) {
        let written = point_shape::spelling(&leaf);
        let full = match leaf.get_ident() {
            Some(ident) => match imports.iter().find(|(name, _)| ident == name) {
                Some((_, path)) => path.clone(),
                None => continue,
            },
            None => written
                .trim_start_matches("::")
                .replace("crate::", "horizon_client::"),
        };
        if found.iter().any(|(seen, _, _)| *seen == written) {
            continue;
        }
        if let Some(api) = wire_type(wire, &full) {
            found.push((written, format!("::{api}"), false));
        } else if let Some((_, api)) = MIRRORS.iter().find(|(path, _)| *path == full)
            && convert
        {
            found.push((written, (*api).to_owned(), true));
        }
    }
    found
}

/// The path `wire` lays out the `mod_api` type at `full` under: the same path, or the one type of
/// that name re-exported elsewhere in `mod_api`.
fn wire_type<'w>(wire: &'w WireTypes, full: &str) -> Option<&'w str> {
    if !full.starts_with("mod_api::") {
        return None;
    }
    if let Some(found) = wire.get(full) {
        return Some(found);
    }
    let name = full.rsplit("::").next()?;
    let mut named = wire
        .iter()
        .filter(|path| path.rsplit("::").next() == Some(name));
    match (named.next(), named.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// Every point, scanned once: each with its file's index.
fn all_points<'a>(
    scanned: &'a [Scanned],
    list: &[(String, String)],
    wire: &WireTypes,
) -> Result<Vec<(usize, Found<'a>)>, String> {
    let mut points = Vec::new();
    for (index, file) in scanned.iter().enumerate() {
        points.extend(
            file.points(list, wire)
                .into_iter()
                .map(|found| (index, found)),
        );
    }
    let mut names = std::collections::BTreeMap::new();
    for (_, found) in &points {
        let shape = found.shape()?;
        for variant in [shape.variant(), shape.return_variant()] {
            if let Some(other) = names.insert(variant.to_string(), found.target()) {
                return Err(format!(
                    "the points of {other} and {} have one name, {}: rename one",
                    found.target(),
                    shape.name
                ));
            }
        }
    }
    Ok(points)
}

/// The game's points, by name: every point function of the game module, and the marked ones of
/// the crates below it. Unmarked targets have no compiled slot.
pub fn game_points(root: &Path) -> Result<Vec<GamePoint>, String> {
    crate::incremental::stamp("before the points");
    let wire = crate::build::wire_types(root)?;
    crate::incremental::stamp("wire types");
    let points = points_with(root, &wire)?;
    crate::incremental::stamp("scan");
    Ok(points)
}

/// [`game_points`] with the `mod_api` types a record may hold.
pub(crate) fn points_with(root: &Path, wire: &WireTypes) -> Result<Vec<GamePoint>, String> {
    if crate::incremental::active() {
        return incremental_points(root, wire);
    }
    let (scanned, list) = scan(root)?;
    let mut points = all_points(&scanned, &list, wire)?
        .into_iter()
        .map(|(_, found)| point_record(found))
        .collect::<Result<Vec<_>, _>>()?;
    points.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(points)
}

/// A file's points as the cache keeps them: each point, with what the check for two points of one
/// name needs.
struct Record {
    /// The name its shape has, for the refusal.
    name: String,
    target: String,
    /// The two variant names its shape declares.
    variants: [String; 2],
    point: GamePoint,
}

/// [`Record`] as stored: a [`GamePoint`] with the fields it does not serialize.
#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    name: String,
    target: String,
    variants: [String; 2],
    api_refusal: Option<String>,
    point_name: String,
    point_target: String,
    side: String,
    host: String,
    record: String,
    args: Vec<(String, String)>,
    returns: String,
    return_ty: String,
    at_return: bool,
    signature_hash: String,
    record_item: String,
    api: [String; 3],
}

impl Stored {
    fn of(record: &Record) -> Self {
        let point = &record.point;
        Stored {
            name: record.name.clone(),
            target: record.target.clone(),
            variants: record.variants.clone(),
            api_refusal: point.api_refusal.clone(),
            point_name: point.name.clone(),
            point_target: point.target.clone(),
            side: point.side.to_owned(),
            host: point.host.to_owned(),
            record: point.record.clone(),
            args: point.args.clone(),
            returns: point.returns.to_owned(),
            return_ty: point.return_ty.clone(),
            at_return: point.at_return,
            signature_hash: point.signature_hash.clone(),
            record_item: point.record_item.clone(),
            api: point.api.clone(),
        }
    }

    /// The record, or none when a word is not one a point has (an entry from another layout).
    fn restore(self) -> Option<Record> {
        fn word(value: &str, words: &[&'static str]) -> Option<&'static str> {
            words.iter().copied().find(|word| *word == value)
        }
        let sides = ["none", "server", "client", "both"];
        Some(Record {
            name: self.name,
            target: self.target,
            variants: self.variants,
            point: GamePoint {
                api_refusal: self.api_refusal,
                name: self.point_name,
                target: self.point_target,
                side: word(&self.side, &sides)?,
                host: word(&self.host, &["none", "host", "server", "client", "both"])?,
                record: self.record,
                args: self.args,
                returns: word(&self.returns, &["none", "cancel", "result"])?,
                return_ty: self.return_ty,
                at_return: self.at_return,
                signature_hash: self.signature_hash,
                record_item: self.record_item,
                api: self.api,
            },
        })
    }
}

/// [`points_with`] for a generation that keeps per-file results ([`crate::incremental`]): each
/// file's points are stored under its content, its module path, the wire types and, for a lower
/// crate, the marked list: everything that decides them. A body edit changes one file's key;
/// the others are read back without being parsed.
fn incremental_points(root: &Path, wire: &WireTypes) -> Result<Vec<GamePoint>, String> {
    use crate::incremental::{cached, digest};
    let (files, list) = inputs(root)?;
    let wire_key = digest(&wire.iter().map(|path| path.as_bytes()).collect::<Vec<_>>());
    let list_key = digest(
        &list
            .iter()
            .map(|(_, path)| path.as_bytes())
            .collect::<Vec<_>>(),
    );
    let mut names = std::collections::BTreeMap::new();
    let mut points = Vec::new();
    for (file, package) in files {
        let source = fs::read_to_string(&file.path).map_err(|e| e.to_string())?;
        let lower = package.is_some();
        let key = digest(&[
            file.module.as_bytes(),
            source.as_bytes(),
            &[u8::from(lower)][..],
            wire_key.as_bytes(),
            // Only a lower crate's points are chosen by the list.
            if lower { list_key.as_bytes() } else { &b""[..] },
        ]);
        let records = cached(
            "points",
            &key,
            || file_records(&file, package, source, &list, wire),
            |records| {
                let stored: Vec<_> = records.iter().map(Stored::of).collect();
                serde_json::to_string(&stored).unwrap_or_default()
            },
            |text| {
                serde_json::from_str::<Vec<Stored>>(text)
                    .ok()?
                    .into_iter()
                    .map(Stored::restore)
                    .collect::<Option<Vec<_>>>()
            },
        )?;
        for record in records {
            for variant in &record.variants {
                if let Some(other) = names.insert(variant.clone(), record.target.clone()) {
                    return Err(format!(
                        "the points of {other} and {} have one name, {}: rename one",
                        record.target, record.name
                    ));
                }
            }
            points.push(record.point);
        }
    }
    points.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(points)
}

/// The points of one file, parsed in full.
fn file_records(
    file: &mod_source::modules::ModuleFile,
    package: Option<PathBuf>,
    source: String,
    list: &[(String, String)],
    wire: &WireTypes,
) -> Result<Vec<Record>, String> {
    let parsed = syn::parse_file(&source).map_err(|e| format!("{}: {e}", file.path.display()))?;
    let scanned = Scanned {
        path: file.path.clone(),
        module: file.module.clone(),
        source,
        parsed,
        package,
    };
    let mut records = Vec::new();
    for found in scanned.points(list, wire) {
        let shape = found.shape()?;
        let variants = [
            shape.variant().to_string(),
            shape.return_variant().to_string(),
        ];
        let name = shape.name.to_string();
        let target = found.target();
        records.push(Record {
            name,
            target,
            variants,
            point: point_record(found)?,
        });
    }
    Ok(records)
}

// Both standalone discovery and source staging retain the original API signature.
fn point_record(found: Found<'_>) -> Result<GamePoint, String> {
    let target = found.target();

    // Replacement normalizes parameter patterns. Keep the API contract of the original
    // signature, including refusals, rather than promoting a staged function into a point.
    let original = found
        .attrs
        .iter()
        .find_map(|attr| {
            if !attr.path().is_ident("doc") {
                return None;
            }
            let syn::Meta::NameValue(value) = &attr.meta else {
                return None;
            };
            let syn::Expr::Lit(value) = &value.value else {
                return None;
            };
            let syn::Lit::Str(value) = &value.lit else {
                return None;
            };
            value
                .value()
                .strip_prefix("horizon-point-signature: ")
                .map(str::to_owned)
        })
        .map(|text| syn::parse_str::<syn::Signature>(&text).map_err(|e| e.to_string()))
        .transpose()?;
    let found = Found {
        sig: original.as_ref().unwrap_or(found.sig),
        ..found
    };
    let shape = found.shape()?;
    Ok(GamePoint::new(shape, &found, target))
}

/// What [`stage`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Staged {
    /// Points given a slot.
    pub points: usize,
    /// Of those, the ones whose API listeners get no return place (their value borrows).
    pub head_only: usize,
    /// Eligible functions left without a point: a parameter has no API type.
    pub refused: usize,
    /// Files rewritten.
    pub files: usize,
}

/// The staging pass: puts `#[point]` on every point function of the staged copy at `root`, in
/// place, and gives each lower crate with points the points' dependencies. Run on a copy, never
/// on a working tree: the source tree carries no `#[point]`.
pub fn stage(root: &Path) -> Result<Staged, String> {
    stage_with(root, &crate::build::wire_types(root)?)
}

/// [`stage`] with the `mod_api` types a record may hold.
pub(crate) fn stage_with(root: &Path, wire: &WireTypes) -> Result<Staged, String> {
    stage_with_manifest(root, wire).map(|(staged, _)| staged)
}

/// Rewrite points and collect their unchanged API records from the same source scan.
pub(crate) fn stage_with_manifest(
    root: &Path,
    wire: &WireTypes,
) -> Result<(Staged, Vec<GamePoint>), String> {
    let (scanned, list) = scan(root)?;
    let points = all_points(&scanned, &list, wire)?;
    let mut records = points
        .iter()
        .map(|(_, found)| point_record(found.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    records.sort_by(|a, b| a.name.cmp(&b.name));
    let mut staged = Staged::default();
    for (index, file) in scanned.iter().enumerate() {
        let here: Vec<_> = points
            .iter()
            .filter(|(i, _)| *i == index)
            .map(|(_, found)| found)
            .collect();
        let source = &file.source;
        let lines: Vec<usize> = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        // Insertions at byte offsets, in order; at one offset, in the order pushed.
        let mut edits: Vec<(usize, String)> = Vec::new();
        for found in &here {
            if let Some(attr) = found.attrs.iter().find(|attr| {
                attr.path()
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "point")
            }) {
                return Err(format!(
                    "{}: {} already has {}: the source carries no #[point]",
                    file.path.display(),
                    found.sig.ident,
                    attr.to_token_stream()
                ));
            }
            let start = match found.vis {
                syn::Visibility::Inherited => found
                    .sig
                    .constness
                    .map(|token| token.span)
                    .unwrap_or(found.sig.fn_token.span)
                    .start(),
                vis => syn::spanned::Spanned::span(vis).start(),
            };
            // A function with no API record has no listeners: it is not marked, and costs nothing.
            let shape = found.shape()?;
            let original = syn::LitStr::new(
                &format!("horizon-point-signature: {}", found.sig.to_token_stream()),
                proc_macro2::Span::call_site(),
            );
            edits.push((
                offset(source, &lines, start),
                format!("{} ", quote!(#[doc = #original])),
            ));
            if !shape.api() {
                staged.refused += 1;
                continue;
            }
            edits.push((
                offset(source, &lines, start),
                format!("#[horizon_mod_api_macros::point({})] ", found.attribute()),
            ));
            staged.points += 1;
            staged.head_only += usize::from(!shape.has_return());
        }
        if edits.is_empty() {
            continue;
        }
        edits.sort_by_key(|(at, _)| *at);
        let mut out = String::with_capacity(
            source.len() + edits.iter().map(|(_, text)| text.len()).sum::<usize>(),
        );
        let mut from = 0;
        for (at, text) in edits {
            out.push_str(&source[from..at]);
            out.push_str(&text);
            from = at;
        }
        out.push_str(&source[from..]);
        fs::write(&file.path, out).map_err(|e| format!("{}: {e}", file.path.display()))?;
        staged.files += 1;
    }
    Ok((staged, records))
}

/// `lock` with `added` in the dependency list of the package `name`, kept in the lock file's
/// sorted order; none when the lock file has no entry for it.
pub(crate) fn locked_with(lock: &str, name: &str, added: &[&str]) -> Option<String> {
    let header = format!("[[package]]\nname = \"{name}\"\n");
    let start = lock.find(&header)?;
    let end = lock[start + header.len()..]
        .find("\n\n")
        .map_or(lock.len(), |at| start + header.len() + at);
    let entry = &lock[start..end];
    let mut lines: Vec<String> = entry.lines().map(str::to_owned).collect();
    let mut dependencies: Vec<String> = Vec::new();
    let open = lines.iter().position(|line| line == "dependencies = [");
    if let Some(open) = open {
        let close = open + lines[open..].iter().position(|line| line == "]")?;
        dependencies = lines.drain(open + 1..close).collect();
    }
    for name in added {
        let quoted = format!(" \"{name}\",");
        if !dependencies.contains(&quoted) {
            dependencies.push(quoted);
        }
    }
    dependencies.sort();
    let open = match open {
        Some(open) => open,
        None => {
            // `dependencies` follows `version` and `source`, before `checksum`.
            let at = lines
                .iter()
                .position(|line| line.starts_with("checksum"))
                .unwrap_or(lines.len());
            lines.insert(at, "dependencies = [".to_owned());
            lines.insert(at + 1, "]".to_owned());
            at
        }
    };
    for (offset, dependency) in dependencies.into_iter().enumerate() {
        lines.insert(open + 1 + offset, dependency);
    }
    Some(format!(
        "{}{}{}",
        &lock[..start],
        lines.join("\n"),
        &lock[end..]
    ))
}

/// Platform `mod_points`: canonical engine-record re-exports, listener constructors
/// and registration place ids. Typed dispatch is owned by `horizon_engine_slots`.
#[cfg(test)]
pub(crate) fn records(points: &[GamePoint]) -> String {
    records_selected(points, None)
}

pub(crate) fn records_selected(
    points: &[GamePoint],
    selected: Option<&std::collections::BTreeSet<String>>,
) -> String {
    let enabled = |point: &GamePoint| {
        selected.is_none_or(|names| {
            names.contains(&point.name) || names.contains(&format!("{}_return", point.name))
        })
    };
    let parse = |code: &str| {
        code.parse::<proc_macro2::TokenStream>()
            .expect("generated points")
    };
    let records = points
        .iter()
        .filter(|point| point.api_refusal.is_none())
        .map(|point| {
            let record = quote::format_ident!("{}", point.record);
            quote!(pub use horizon_engine_slots::#record;)
        });
    let listeners = points.iter().map(|point| parse(&point.api[0]));
    let variants: Vec<_> = points.iter().map(|point| parse(&point.api[1])).collect();
    let mut guest_functions = proc_macro2::TokenStream::new();
    let mut guest_entries = proc_macro2::TokenStream::new();
    let targets: Vec<_> = points
        .iter()
        .flat_map(|p| {
            let variants = parse(&p.api[1]);
            let ids: syn::ItemEnum =
                syn::parse2(quote!(enum Places { #variants })).expect("point variants");
            std::iter::repeat_n(p.target.clone(), ids.variants.len())
        })
        .collect();
    let mut api_places = proc_macro2::TokenStream::new();
    let mut api_refusals = proc_macro2::TokenStream::new();
    let mut layout = 0xcbf29ce484222325u64;
    for point in points {
        for byte in format!(
            "{}:{}:{}:{}:{:?}",
            point.name, point.host, point.record_item, point.at_return, point.api_refusal
        )
        .bytes()
        {
            layout = (layout ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        if let Some(reason) = &point.api_refusal {
            let target = &point.target;
            api_refusals.extend(quote! { (#target, #reason), });
            continue;
        }
        let record = quote::format_ident!("{}", point.record);
        let passed = match point.host {
            "host" => quote!(&host,),
            "server" | "client" => quote!(&mut host,),
            "both" => quote!(if server {
                ::mod_api::HostSide::Server(&mut host)
            } else {
                ::mod_api::HostSide::Client(&mut host)
            },),
            _ => quote!(),
        };
        let function = quote::format_ident!("point_{}", point.name);
        let listener = quote::format_ident!("{}Listener", point_shape::camel(&point.name));
        let side = (point.host == "both").then(|| quote!(let server: bool = input.take()?;));
        if enabled(point) {
            guest_functions.extend(quote! {
            fn #function(place: PointId, input: &mut ::mod_api::wire::Reader<'_>, out: &mut ::mod_api::wire::Writer) -> Result<(), ::mod_api::wire::WireError> {
                let listener: u32 = input.take()?;
                #side
                let mut args: #record = input.take()?;
                let mut host = ::mod_api::wire::guest::Guest;
                crate::guest::fire(place, listener, |function| {
                    // SAFETY: head and return have the same checked host and record type.
                    let function = unsafe { ::std::mem::transmute::<*const (), listeners::#listener>(function) };
                    function(#passed &mut args);
                })?;
                out.put(&());
                out.put(&args);
                Ok(())
            }
        });
        }
        for name in std::iter::once(point.name.clone())
            .chain(point.at_return.then(|| format!("{}_return", point.name)))
        {
            let variant = quote::format_ident!("{}", point_shape::camel(&name));
            let target = &point.target;
            let at_return = name != point.name;
            let replace_refusal = match point.replace_refusal().filter(|_| !at_return) {
                Some(ty) => quote!(::std::option::Option::Some(#ty)),
                None => quote!(::std::option::Option::None),
            };
            api_places
                .extend(quote! { (PointId::#variant, #target, #at_return, #replace_refusal), });
            if selected.is_some_and(|names| !names.contains(&name)) {
                continue;
            }
            guest_entries.extend(quote! {
                table[PointId::#variant as usize] = Some((PointId::#variant, #function as Dispatch));
            });
        }
    }
    let ids: syn::ItemEnum = syn::parse2(quote! {
        enum PointId { #(#variants)* __Count }
    })
    .expect("point variants");
    let debug_names = ids.variants.iter().map(|variant| variant.ident.to_string());
    quote! {
        #(#records)*
        /// Target names for native load diagnostics.
        #[doc(hidden)]
        pub const POINT_TARGETS: &[&str] = &[#(#targets),*];
        /// The generated point export table and record layout, checked separately from the API layout.
        #[doc(hidden)]
        pub const POINT_LAYOUT: u64 = #layout;
        /// API places as (place, target function, return phase, the return type a `replace` there
        /// can't produce), for manifest validation: a head whose return is neither `()` nor an
        /// API value refuses a replacement when a module loads.
        #[doc(hidden)]
        /// Targets whose arguments cannot cross the API/Wire boundary, with the named type.
        #[doc(hidden)]
        pub const API_REFUSALS: &[(&str, &str)] = &[#api_refusals];
        #[doc(hidden)]
        pub const API_PLACES: &[(PointId, &str, bool, Option<&str>)] = &[#api_places];
        #[cfg(any(target_arch = "wasm32", feature = "wasm-guest"))]
        #[doc(hidden)]
        pub fn guest_dispatch(id: u32, input: &mut ::mod_api::wire::Reader<'_>, out: &mut ::mod_api::wire::Writer) -> Result<(), ::mod_api::wire::WireError> {
            if id == 0xBFFF_FFFF { out.put(&crate::guest::subscriptions()); return Ok(()); }
            let index = (id & 0x3FFF_FFFF) as usize;
            let (place, function) = guest_exports::TABLE.get(index).copied().flatten().ok_or(::mod_api::wire::WireError::Tag(id))?;
            function(place, input, out)
        }
        #[cfg(any(target_arch = "wasm32", feature = "wasm-guest"))]
        #[allow(unused_mut, unused_variables)]
        mod guest_exports {
            use super::*;
            type Dispatch = fn(PointId, &mut ::mod_api::wire::Reader<'_>, &mut ::mod_api::wire::Writer) -> Result<(), ::mod_api::wire::WireError>;
            pub(super) static TABLE: [Option<(PointId, Dispatch)>; POINT_COUNT] = {
                let mut table = [None; POINT_COUNT];
                #guest_entries
                table
            };
            #guest_functions
        }
        /// A point's place: its head and return in catalog order.
        #[doc(hidden)]
        #[repr(usize)]
        #[derive(Clone, Copy, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum PointId { #(#variants)* __Count }
        // Deriving Debug creates an enormous match for this enum. Keep the exact
        // unit-variant names without its compiler-memory cost.
        impl ::std::fmt::Debug for PointId {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                static NAMES: [&str; PointId::__Count as usize + 1] = [#(#debug_names),*];
                formatter.write_str(NAMES[*self as usize])
            }
        }
        /// The number of places, the length of the game's slot table.
        #[doc(hidden)]
        pub const POINT_COUNT: usize = PointId::__Count as usize;
        /// Typed API listener aliases and constructors, generated from the point catalog.
        pub mod listeners { use super::*; #(#listeners)* }
    }
    .to_string()
}

/// Engine-owned records and typed dispatch; registration stays in the platform.
pub(crate) fn engine_records(
    points: &[GamePoint],
    selected: Option<&std::collections::BTreeSet<String>>,
) -> String {
    let parse = |code: &str| {
        code.parse::<proc_macro2::TokenStream>()
            .expect("generated points")
    };
    let records = points.iter().map(|point| {
        let mut file: syn::File = syn::parse_str(&point.record_item).expect("point record");
        if selected.is_some_and(|names| {
            !names.contains(&point.name) && !names.contains(&format!("{}_return", point.name))
        }) {
            for item in &mut file.items {
                if let syn::Item::Struct(record) = item {
                    record.attrs.retain(|attr| {
                        !(attr.path().is_ident("cfg_attr")
                            && attr
                                .to_token_stream()
                                .to_string()
                                .contains("derive (:: mod_api :: Wire)"))
                    });
                }
            }
        }
        quote!(#file)
    });
    let variants: Vec<_> = points.iter().map(|point| parse(&point.api[1])).collect();
    let dispatch = points.iter().map(|point| parse(&point.api[2]));
    let listeners = points.iter().map(|point| {
        let items = parse(&point.api[0]);
        let mut module: syn::ItemMod =
            syn::parse2(quote!(mod listeners { #items })).expect("point listeners");
        module
            .content
            .as_mut()
            .unwrap()
            .1
            .retain(|item| matches!(item, syn::Item::Type(_)));
        let items = &module.content.as_ref().unwrap().1;
        quote!(#(#items)*)
    });
    quote! {
        #(#records)*
        /// Catalog-ordered engine point places.
        #[repr(usize)]
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub enum PointId { #(#variants)* __Count }
        /// Number of engine point places.
        pub const POINT_COUNT: usize = PointId::__Count as usize;
        /// Checked function types used by typed dispatch.
        pub mod listeners { use super::*; #(#listeners)* }
        /// Typed dispatch through the installed provider.
        pub mod dispatch { use super::*; #(#dispatch)* }
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_points_signature_hash_follows_its_signature_not_its_name() {
        let point = |name: &str, ty: &str| {
            GamePoint {
                api_refusal: None,
                name: name.into(),
                target: "game::tick".into(),
                side: "server",
                host: "server",
                record: "TickRecord".into(),
                args: vec![("count".into(), ty.into())],
                returns: "cancel",
                return_ty: "()".into(),
                at_return: false,
                signature_hash: String::new(),
                record_item: String::new(),
                api: Default::default(),
            }
            .hashed()
            .signature_hash
        };
        let base = point("tick", "u32");
        assert_eq!(base.len(), 32);
        assert_eq!(point("renamed", "u32"), base);
        assert_ne!(point("tick", "u64"), base);
    }

    #[test]
    fn selected_guest_exports_keep_the_full_layout() {
        let make = |name: &str| {
            let sig: syn::Signature = syn::parse_str(&format!("fn {name}()")).unwrap();
            let shape = point_shape::shape(quote!(), &sig).unwrap();
            let api = shape.api_items(name);
            GamePoint {
                api_refusal: None,
                name: name.into(),
                target: name.into(),
                side: "none",
                host: "none",
                record: shape.record().to_string(),
                args: vec![],
                returns: "cancel",
                return_ty: "()".into(),
                at_return: true,
                signature_hash: String::new(),
                record_item: shape.record_item(name).to_string(),
                api: [
                    api.listeners.to_string(),
                    api.variants.to_string(),
                    api.dispatch.to_string(),
                ],
            }
        };
        let points = [make("first"), make("second")];
        let selected = std::collections::BTreeSet::from(["first_return".to_string()]);
        let engine = engine_records(&points, Some(&selected));
        let all = records(&points);
        let selected = records_selected(&points, Some(&selected));
        let layout = |source: &str| {
            syn::parse_file(source)
                .unwrap()
                .items
                .into_iter()
                .find_map(|item| match item {
                    syn::Item::Const(c) if c.ident == "POINT_LAYOUT" => {
                        Some(c.expr.to_token_stream().to_string())
                    }
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(layout(&all), layout(&selected));
        let file = syn::parse_file(&engine).unwrap();
        let wires = file.items.iter().filter(|item| matches!(item, syn::Item::Struct(record) if record.attrs.iter().any(|attr| attr.path().is_ident("cfg_attr") && attr.to_token_stream().to_string().contains("derive (:: mod_api :: Wire)")))).count();
        assert_eq!(wires, 1);
        assert!(selected.contains("fn point_first"));
        assert!(!selected.contains("fn point_second"));
        assert!(selected.contains("table [PointId :: FirstReturn as usize]"));
        assert!(!selected.contains("table [PointId :: First as usize]"));
    }

    /// The census: how many points have an API record, and why the others refuse.
    /// `POINT_INVENTORY=path` writes every point's name, target and refusal.
    #[test]
    #[ignore = "daily"]
    fn point_inventory() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let points = game_points(&root).unwrap();
        if let Ok(path) = std::env::var("POINT_INVENTORY") {
            let inventory: Vec<_> = points
                .iter()
                .map(|point| {
                    serde_json::json!({
                        "name": point.name,
                        "target": point.target,
                        "api_refusal": point.api_refusal,
                    })
                })
                .collect();
            fs::write(path, serde_json::to_vec_pretty(&inventory).unwrap()).unwrap();
        }
        let api = points.iter().filter(|p| p.api_refusal.is_none()).count();
        println!("{} points; {api} with an API record", points.len());
        let mut refused = std::collections::BTreeMap::<String, usize>::new();
        for reason in points.iter().filter_map(|p| p.api_refusal.as_deref()) {
            // `argument 2 `x` (`T`) has no API type yet`: count by type.
            let key = reason
                .split_once("(`")
                .and_then(|(_, rest)| rest.split_once("`)"))
                .map_or(reason, |(ty, _)| ty);
            *refused.entry(key.to_owned()).or_default() += 1;
        }
        let mut refused: Vec<_> = refused.into_iter().collect();
        refused.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        for (reason, count) in refused.iter().take(60) {
            println!("refused {count}: {reason}");
        }
        assert!(api > 0);
    }

    #[test]
    #[ignore = "daily"]
    fn no_wire_public_values_refuse_before_point_record_emission() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let wire = crate::build::wire_types(&root).unwrap();
        for name in [
            "EnchantmentEffect",
            "RegistrationData",
            "Parts",
            "Attachments",
            "AttachmentAccess",
        ] {
            let method: syn::ImplItemFn =
                syn::parse_str(&format!("fn patch(&mut self, value: &{name}) {{}}")).unwrap();
            let imports = vec![(name.to_owned(), format!("mod_api::{name}"))];
            assert!(
                super::mirrors(&method.sig, &imports, &wire, false).is_empty(),
                "{name} must not be admitted by Clone alone"
            );
            let shape = point_shape::shape(quote!(side = "server"), &method.sig).unwrap();
            assert!(shape.api_refusal.as_ref().unwrap().contains(name));
            assert!(shape.api_items("test").listeners.is_empty());
        }
        for name in [
            "mod_api::ConnectionKind",
            "mod_api::item::containers::ModRevision",
            "mod_api::client::camera::UseItem",
            "mod_api::wire::Id",
        ] {
            assert!(wire.contains(name), "supported public path {name} was lost");
        }
        assert!(!wire.contains("mod_api::wire::wire_slots::Id"));
    }

    fn point(attribute: &str, method: &str) -> Result<point_shape::Shape, String> {
        let method: syn::ImplItemFn = syn::parse_str(method).map_err(|e| e.to_string())?;
        point_shape::shape(attribute.parse().unwrap(), &method.sig).map_err(|e| e.to_string())
    }

    fn eligible(method: &str) -> bool {
        let method: syn::ImplItemFn = syn::parse_str(method).unwrap();
        point_shape::eligible(&method.attrs, &method.sig)
    }

    #[test]
    fn a_point_passes_every_parameter_or_refuses_naming_it() {
        let shape = point(
            r#"side = "client""#,
            "fn turn(&mut self, mut yaw: f64, text: &str, slots: &[u32], count: &mut usize, names: Vec<String>) {}",
        )
        .unwrap();
        let args: Vec<_> = shape
            .args
            .iter()
            .map(|arg| (arg.name.to_string(), arg.pass))
            .collect();
        use point_shape::Pass;
        assert_eq!(
            args,
            [
                ("yaw".into(), Pass::Value),
                ("text".into(), Pass::Shared),
                ("slots".into(), Pass::Shared),
                ("count".into(), Pass::Mut),
                ("names".into(), Pass::Value)
            ]
        );
        assert!(matches!(shape.returns, point_shape::Returns::Unit));
        let item = shape.record_item("t").to_string();
        assert!(
            item.contains("pub yaw : f64")
                && item.contains("pub text : :: std :: borrow :: Cow < 'a , str >")
                && item.contains("pub slots : :: std :: vec :: Vec < u32 >")
                && item.contains("pub count : u64")
                && item.contains("pub names : :: std :: vec :: Vec < :: std :: string :: String >")
                && item.contains("pub cancel : bool"),
            "{item}"
        );
        for (signature, reason) in [
            (
                "fn turn(&mut self, yaw: f64, batch: &mut HudBatch) {}",
                "argument 2 `batch` (`&mutHudBatch`) has no API type yet",
            ),
            (
                "fn turn(&mut self, event: &'static str) {}",
                "argument 1 `event` (`&'staticstr`) has no API type yet",
            ),
            (
                "fn turn(&mut self, (a, b): (u8, u8)) {}",
                "argument 1 is a pattern, not a name",
            ),
        ] {
            let shape = point(r#"side = "client""#, signature).unwrap();
            assert_eq!(shape.api_refusal.as_deref(), Some(reason), "{signature}");
            assert!(shape.api_items("t").listeners.is_empty());
        }
        let mirrored = point(
            r#"side = "both", mirror(Text = "::mod_api::Text")"#,
            "fn say(&mut self, line: &Text, lines: Option<Vec<Text>>) -> Text {}",
        )
        .unwrap();
        assert!(mirrored.api_refusal.is_none());
        assert!(mirrored.args.iter().all(|arg| arg.mirror));
        let item = mirrored.record_item("t").to_string();
        assert!(
            item.contains("pub line : :: mod_api :: Text")
                && item.contains(
                    "pub lines : :: std :: option :: Option < :: std :: vec :: Vec < :: mod_api :: Text > >"
                )
                && item.contains("pub result : Option < :: mod_api :: Text >"),
            "{item}"
        );
        // A receiver other than `Game` has no API handle yet.
        let other = point(r#"name = "boat_paddle""#, "fn paddle(&mut self) {}").unwrap();
        assert_eq!(
            other.api_refusal.as_deref(),
            Some("its `self` has no API handle yet")
        );
        let read = point(
            r#"side = "server", name = "reach_of""#,
            "fn reach(&self) -> f64 { 0.0 }",
        )
        .unwrap();
        assert_eq!(
            (read.name.as_str(), read.variant().to_string()),
            ("reach_of", "ReachOf".into())
        );
        assert!(
            read.record_item("t")
                .to_string()
                .contains("pub result : Option < f64 >")
        );
    }

    #[test]
    fn what_a_listener_may_do_follows_the_return_type() {
        let returns = |sig: &str| point(r#"side = "both""#, sig).map(|shape| shape.returns);
        assert!(matches!(
            returns("fn tick(&mut self) -> Result<()> {}"),
            Ok(point_shape::Returns::UnitResult)
        ));
        assert!(matches!(
            returns("fn hit(&self) -> bool {}"),
            Ok(point_shape::Returns::Value(_))
        ));
        assert!(matches!(
            returns("fn hit(&self) -> Option<(usize, f64)> {}"),
            Ok(point_shape::Returns::Value(_))
        ));
        assert!(matches!(
            returns("fn hit(&self) -> Option<HitResult> {}"),
            Ok(point_shape::Returns::Opaque)
        ));
        let at_return = |sig: &str| point(r#"side = "both""#, sig).unwrap().has_return();
        assert!(at_return("fn hit(&self) -> Option<(usize, f64)> {}"));
        assert!(!at_return(
            "fn slot(&mut self, i: u32) -> Option<&mut ItemStack> {}"
        ));
        assert!(!at_return("fn view(&self) -> View<'_> {}"));
        // A `'static` return borrows nothing from the receiver.
        assert!(at_return("fn jab(&self) -> Option<&'static str> {}"));
    }

    #[test]
    fn every_plain_game_method_is_a_point() {
        assert!(eligible("fn tick(&mut self) -> Result<()> {}"));
        assert!(eligible("pub(super) fn reach(&self) -> f64 { 0.0 }"));
        assert!(eligible("fn get<T: Copy>(&self) -> T { todo!() }"));
        // An associated or free function is a point too; off `Game`, with no host.
        assert!(eligible("fn new() -> Self {}"));
        for left_out in [
            "fn native_server_tick(&mut self) {}",
            "fn take(self) {}",
            "const fn size(&self) -> usize { 0 }",
            "async fn wait(&self) {}",
            "#[cfg(test)] fn probe(&self) {}",
            "#[inline(always)] fn hot(&self) {}",
            "fn each(&self, f: impl Fn()) {}",
            "fn iter(&self) -> impl Iterator<Item = u8> { todo!() }",
            "fn stop(&self) -> ! { loop {} }",
            "fn f(&mut self, cancel: bool) {}",
        ] {
            assert!(!eligible(left_out), "{left_out}");
        }
    }

    /// Two functions whose paths snake-case to one name are refused at build, naming both:
    /// `a::b_c` and `a_b::c` both give `game_a_b_c`.
    #[test]
    fn engine_points_with_one_snake_cased_name_are_refused_naming_both() {
        let root =
            std::env::temp_dir().join(format!("horizon-points-clash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let put = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        put("crates/horizon-mod-api-gen/internals.txt", "");
        put(
            "crates/horizon-client/Cargo.toml",
            "[package]
name = 'horizon-client'
",
        );
        put(
            "crates/horizon-client/src/lib.rs",
            "mod game;
",
        );
        put(
            "crates/horizon-client/src/game.rs",
            "mod a;
mod a_b;
",
        );
        put(
            "crates/horizon-client/src/game/a.rs",
            "fn b_c() {}
",
        );
        put(
            "crates/horizon-client/src/game/a_b.rs",
            "fn c() {}
",
        );
        put(
            "crates/horizon-engine-slots/Cargo.toml",
            "[package]
name = 'horizon-engine-slots'
[dependencies]
horizon-world = { path = '../horizon-world' }
[build-dependencies]
horizon-mod-api-gen = { path = '../horizon-mod-api-gen' }
",
        );
        put(
            "crates/horizon-world/Cargo.toml",
            "[package]
name = 'horizon-world'
",
        );
        put(
            "crates/horizon-mod-api-gen/Cargo.toml",
            "[package]
name = 'horizon-mod-api-gen'
",
        );
        let error = points_with(&root, &WireTypes::new()).unwrap_err();
        assert!(
            error.contains("horizon_client::game::a::b_c")
                && error.contains("horizon_client::game::a_b::c")
                && error.contains("game_a_b_c"),
            "{error}"
        );
        // Without the second function the same tree is accepted.
        put("crates/horizon-client/src/game/a_b.rs", "");
        assert!(points_with(&root, &WireTypes::new()).is_ok());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn replacement_keeps_destructured_parameter_point_layout() {
        let root =
            std::env::temp_dir().join(format!("horizon-point-pattern-{}", std::process::id()));
        let put = |name: &str, text: &str| {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        for name in [
            "horizon-client",
            "horizon-engine-slots",
            "horizon-mod-api-gen",
        ] {
            put(
                &format!("crates/{name}/Cargo.toml"),
                &format!("[package]\nname = '{name}'\n"),
            );
        }
        put("crates/horizon-mod-api-gen/internals.txt", "");
        put("crates/horizon-mod-api-gen/replaceable.txt", "");
        put("crates/horizon-client/src/lib.rs", "mod game;");
        put(
            "crates/horizon-client/src/game.rs",
            "impl Game<'_> { fn connect_cancel(&self, (w, h): (f32, f32)) -> Option<(f32, f32)> { None } }",
        );
        let before = points_with(&root, &WireTypes::new()).unwrap();
        assert!(before[0].api_refusal.is_some());
        let layout = records(&before);
        let (_, combined) = stage_with_manifest(&root, &WireTypes::new()).unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&combined).unwrap()
        );
        assert_eq!(layout, records(&combined));
        crate::replace::stage(&root).unwrap();
        let after = points_with(&root, &WireTypes::new()).unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        assert_eq!(layout, records(&after));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn marked_crates_need_slots_except_below_slots() {
        let root =
            std::env::temp_dir().join(format!("horizon-point-dependency-{}", std::process::id()));
        let put = |path: &str, text: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        put("crates/horizon-client/src/game.rs", "");
        put(
            "crates/horizon-mod-api-gen/Cargo.toml",
            "[package]\nname = 'horizon-mod-api-gen'\n",
        );
        put("crates/horizon-mod-api-gen/internals.txt", "");
        put(
            "crates/horizon-engine-slots/Cargo.toml",
            "[package]\nname = 'horizon-engine-slots'\n[dependencies]\nhorizon-world = { path = '../horizon-world' }\n",
        );
        put(
            "crates/horizon-world/Cargo.toml",
            "[package]\nname = 'horizon-world'\n",
        );
        put(
            "crates/horizon-world/src/lib.rs",
            "#[horizon_mod_api_macros::replaceable] pub fn lower() {}\n",
        );
        let manifest = "crates/horizon-client-render/Cargo.toml";
        let package = "[package]\nname = 'horizon-client-render'\n";
        put(manifest, package);
        put(
            "crates/horizon-client-render/src/lib.rs",
            "#[horizon_mod_api_macros::replaceable] pub fn mesh() {}\n",
        );
        assert_eq!(
            scan(&root).err().unwrap(),
            "crate horizon_client_render has points but does not depend on horizon-engine-slots"
        );
        put(
            "Cargo.toml",
            "[workspace.dependencies]\nslots = { package = 'horizon-engine-slots', path = 'crates/horizon-engine-slots' }\n",
        );
        for dependency in [
            "[dependencies]\nhorizon-engine-slots = { path = '../horizon-engine-slots' }\n",
            "[dependencies]\nslots = { package = 'horizon-engine-slots', path = '../horizon-engine-slots' }\n",
            "[dependencies]\nslots = { workspace = true }\n",
            "[target.'cfg(unix)'.dependencies]\nslots = { package = 'horizon-engine-slots', path = '../horizon-engine-slots' }\n",
        ] {
            put(manifest, &format!("{package}{dependency}"));
            let (files, _) = scan(&root).unwrap();
            assert!(
                files
                    .iter()
                    .any(|file| file.module == "horizon_client_render")
            );
            assert!(!files.iter().any(|file| file.module == "horizon_world"));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_pass_marks_the_functions_the_generator_lists() {
        let root = std::env::temp_dir().join(format!("horizon-points-{}", std::process::id()));
        let put = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        put("crates/horizon-mod-api-gen/internals.txt", "");
        put(
            "crates/horizon-client/Cargo.toml",
            "[package]\nname = 'horizon-client'\n",
        );
        put("crates/horizon-client/src/lib.rs", "mod game; mod entry;\n");
        put("crates/horizon-client/src/entry.rs", "");
        put(
            "crates/horizon-client/src/game.rs",
            "mod remote;\nmod nested { mod leaf; }\n#[path = \"elsewhere\"]\nmod moved { mod deep; }\n#[cfg(test)]\nmod probes;\nimpl Game<'_> {\n    fn tick(&mut self) {}\n    pub(super) fn slot(&mut self) -> &mut u8 { todo!() }\n    fn native_tick(&mut self) {}\n    fn new() -> Self { todo!() }\n}\n",
        );
        put(
            "crates/horizon-client/src/game/remote.rs",
            "mod lan;\nimpl Game<'_> { fn send(&self, id: u32) {} }\nimpl Other { fn send(&self) {} }\nfn decode<T>(bytes: &[u8]) -> T { todo!() }\n",
        );
        put(
            "crates/horizon-client/src/game/remote/lan.rs",
            "impl Game<'_> { fn join(&mut self) {} }",
        );
        put(
            "crates/horizon-client/src/game/elsewhere/deep.rs",
            "impl Game<'_> { fn dive(&mut self) {} }",
        );
        put(
            "crates/horizon-client/src/game/nested/leaf.rs",
            "impl Game<'_> { fn fall(&mut self) {} }",
        );
        put(
            "crates/horizon-client/src/game/probes.rs",
            "impl Game<'_> { fn probe(&self) {} }",
        );
        put(
            "crates/horizon-engine-slots/Cargo.toml",
            "[package]\nname = 'horizon-engine-slots'\n[dependencies]\nhorizon-world = { path = '../horizon-world' }\n[build-dependencies]\nhorizon-mod-api-gen = { path = '../horizon-mod-api-gen' }\n",
        );
        put(
            "crates/horizon-world/Cargo.toml",
            "[package]\nname = 'horizon-world'\n",
        );
        put(
            "crates/horizon-mod-api-gen/Cargo.toml",
            "[package]\nname = 'horizon-mod-api-gen'\n",
        );
        put(
            "crates/horizon-client-render/Cargo.toml",
            "[package]\nname = 'horizon-client-render'\n[dependencies]\nlog = '0.4'\nhorizon-engine-slots = { path = '../horizon-engine-slots' }\n",
        );
        put(
            "Cargo.lock",
            // CRLF, as a Windows checkout writes it.
            &"version = 4\n\n[[package]]\nname = \"horizon-client\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"horizon-client-render\"\nversion = \"0.1.0\"\ndependencies = [\n \"log\",\n]\n\n[[package]]\nname = \"log\"\nversion = \"0.4.0\"\n"
                .replace('\n', "\r\n"),
        );
        put(
            "crates/horizon-client-render/src/lib.rs",
            "pub mod mesher;\n",
        );
        put(
            "crates/horizon-client-render/src/mesher.rs",
            "pub struct Mesher<'a>(&'a u8);\nimpl Mesher<'_> { #[horizon_mod_api_macros::replaceable] pub fn mesh(&mut self, chunk: &Chunk) -> usize { 0 } fn face(&self) {} }\n",
        );
        let points = points_with(&root, &WireTypes::new()).unwrap();
        let named: Vec<_> = points
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.target.as_str(),
                    p.side,
                    p.api_refusal.is_none(),
                )
            })
            .collect();
        assert_eq!(
            named,
            [
                (
                    "client_render_mesher_mesher_mesh",
                    "horizon_client_render::mesher::Mesher<'_>::mesh",
                    "none",
                    false
                ),
                (
                    "dive",
                    "horizon_client::game::moved::deep::Game<'_>::dive",
                    "both",
                    true
                ),
                (
                    "fall",
                    "horizon_client::game::nested::leaf::Game<'_>::fall",
                    "both",
                    true
                ),
                (
                    "game_game_new",
                    "horizon_client::game::Game<'_>::new",
                    "none",
                    true
                ),
                (
                    "game_remote_decode",
                    "horizon_client::game::remote::decode",
                    "none",
                    false
                ),
                (
                    "game_remote_other_send",
                    "horizon_client::game::remote::Other::send",
                    "none",
                    false
                ),
                (
                    "join",
                    "horizon_client::game::remote::lan::Game<'_>::join",
                    "both",
                    true
                ),
                (
                    "send",
                    "horizon_client::game::remote::Game<'_>::send",
                    "both",
                    true
                ),
                ("slot", "horizon_client::game::Game<'_>::slot", "both", true),
                ("tick", "horizon_client::game::Game<'_>::tick", "both", true),
            ]
        );
        let api = records(&points);
        assert!(
            api.contains("fn tick_return")
                // A borrowed return has no return place.
                && !api.contains("fn slot_return")
                // `Other` has no API handle: it has no point.
                && !api.contains("fn game_remote_other_send ("),
            "{api}"
        );
        // A generic function takes no replacement, so it has no site: refused, naming the rule.
        let decode = points.iter().find(|p| p.name == "game_remote_decode");
        assert_eq!(
            decode.unwrap().api_refusal.as_deref(),
            Some(
                "the function has no point site: it takes no replacement (function type/const generics)"
            )
        );
        let (staged, combined) = stage_with_manifest(&root, &WireTypes::new()).unwrap();
        let after = points_with(&root, &WireTypes::new()).unwrap();
        // Compare every manifest field and the nonserialized API layout, not just counts.
        for actual in [&combined, &after] {
            assert_eq!(
                serde_json::to_value(&points).unwrap(),
                serde_json::to_value(actual).unwrap()
            );
            assert_eq!(api, records(actual));
            for (expected, actual) in points.iter().zip(actual) {
                assert_eq!(expected.record_item, actual.record_item);
                assert_eq!(expected.api, actual.api);
            }
        }
        assert_eq!((staged.points, staged.head_only, staged.refused), (7, 1, 3));
        let generated = root.join("staged-generated");
        std::fs::create_dir_all(&generated).unwrap();
        // Only the stager's stamp on the source copy makes a build one with point sites.
        for staged in [false, true] {
            if staged {
                std::fs::write(root.join(".horizon-source-staged"), "").unwrap();
            }
            crate::build::generate_points_inner(&root, &generated).unwrap();
            let code = std::fs::read_to_string(generated.join("points_generated.rs")).unwrap();
            assert_eq!(
                code.contains("pub const STAGED: bool = true"),
                staged || cfg!(feature = "listener-fixture")
            );
        }
        let game = std::fs::read_to_string(root.join("crates/horizon-client/src/game.rs")).unwrap();
        assert!(
            game.contains("#[horizon_mod_api_macros::point(side = \"both\")] fn tick(")
                && game.contains("#[horizon_mod_api_macros::point(side = \"both\")] pub(super) fn slot(")
                && !game.contains(")] fn native_tick")
                // `Game`'s associated functions are named and hosted like any other type's.
                && game.contains("point(name = \"game_game_new\""),
            "{game}"
        );
        // A function the API can't carry, or that takes no replacement, is not marked.
        let remote =
            std::fs::read_to_string(root.join("crates/horizon-client/src/game/remote.rs")).unwrap();
        assert!(
            !remote.contains("game_remote_decode") && !remote.contains("game_remote_other_send"),
            "{remote}"
        );
        // A lower crate with no marked function keeps its manifest.
        let render = root.join("crates/horizon-client-render");
        let mesher = std::fs::read_to_string(render.join("src/mesher.rs")).unwrap();
        assert!(!mesher.contains("point("), "{mesher}");
        let manifest = std::fs::read_to_string(render.join("Cargo.toml")).unwrap();
        assert!(!manifest.contains("horizon-mod-points"), "{manifest}");
        // Staging leaves the engine manifest and lock dependency graph unchanged.
        let client =
            std::fs::read_to_string(root.join("crates/horizon-client/Cargo.toml")).unwrap();
        assert!(!client.contains("horizon-mod-points"), "{client}");
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        assert!(!lock.contains(" \"horizon-mod-points\","), "{lock}");
        // A second pass finds the attribute and refuses: the source tree never carries one.
        assert!(
            stage_with(&root, &WireTypes::new())
                .unwrap_err()
                .contains("already has")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A module whose file each target names with `cfg_attr(…, path = …)` is read from each
    /// target's file under its condition; it has no default file, and none is needed.
    #[test]
    fn per_target_module_paths_are_followed_and_need_no_default_file() {
        let dir = std::env::temp_dir().join(format!("points-cfg-attr-{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("control")).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "#[cfg_attr(unix, path = \"control/unix.rs\")]\n#[cfg_attr(windows, path = \"control/windows.rs\")]\nmod platform;\n",
        )
        .unwrap();
        std::fs::write(src.join("control/unix.rs"), "pub fn a() {}").unwrap();
        std::fs::write(src.join("control/windows.rs"), "pub fn b() {}").unwrap();
        let files = crate::source::module_files("k".into(), src.join("lib.rs"), src.clone(), false)
            .unwrap();
        let mut found: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.module.as_str(),
                    f.path.strip_prefix(&src).unwrap().to_owned(),
                    f.cfg.clone(),
                )
            })
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                ("k", "lib.rs".into(), vec![]),
                (
                    "k::platform",
                    "control/unix.rs".into(),
                    vec!["unix".to_owned()]
                ),
                (
                    "k::platform",
                    "control/windows.rs".into(),
                    vec!["windows".to_owned()]
                ),
            ]
        );
        // Every build compiles neither, so the point walk leaves the module out.
        let every = crate::source::module_files("k".into(), src.join("lib.rs"), src, true).unwrap();
        assert_eq!(every.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
