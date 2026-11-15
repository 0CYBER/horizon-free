//! Function replacement for `[[native.mixin]]` (fn-replace): the patch crate a mixin build compiles
//! against the prebuilt game instead of rebuilding it. Each target function is copied, unchanged,
//! out of the pinned source into a module that mirrors its own (`patch::game::autosave` for
//! `horizon_client::game::autosave`), where every name the body uses resolves as it did in the
//! game: the mirror re-exports its module's items from the prebuilt game, which makes them all
//! public, and repeats the module's own `use` items, so `self`, `super::` and `crate::` mean what
//! they meant. A method moves into a one-method trait implemented for its own type, so `self`,
//! `Self` and the helpers' bindings stay as written. horizon-mixins then patches the copies, and
//! the game installs each through its function's replacement slot (`__fnr_<name>`).
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::{ImplItem, Item};

/// One target function, copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patched {
    /// The target as the `[[native.mixin]]` row writes it.
    pub target: String,
    /// The copy's id for horizon-mixins, in the patch crate.
    pub id: String,
    /// The expression that builds the copy's installation result, from the patch crate's root;
    /// none for a published loop's generic function, which has no cell: its entry's copy calls it.
    pub install: Option<String>,
}

/// The patch crate's library source and the copies in it.
#[derive(Debug)]
pub struct PatchCrate {
    /// One library per game crate with copies, `(crate name, source)`: a copy's `crate::` paths
    /// resolve against the mirror of its own crate.
    pub libs: Vec<(String, String)>,
    pub patched: Vec<Patched>,
}

/// A `[[native.module]]`: Rust items emitted as a module of the mirror of the module `file`
/// holds, where the copies reach them by the paths they would have had in the game.
#[derive(Debug, Clone)]
pub struct SupportModule {
    pub id: String,
    /// The game source file, relative to the source root, as the row names it.
    pub file: String,
    pub name: String,
    /// The payload's text.
    pub text: String,
    pub public: bool,
}

/// A `[[native.bridge]]`: horizon-mixins' accessor or invoker, emitted as an extension trait the
/// mirror modules import, so the helpers' method calls resolve as they did on the game's type.
#[derive(Debug, Clone)]
pub struct Bridge {
    pub id: String,
    /// The generated method's name, `__horizon_mixins_<mod>_<id>`.
    pub method: String,
    /// `GET`, `GET_MUT`, `SET` or `INVOKE`.
    pub kind: String,
    pub target: String,
    pub member: Option<String>,
}

/// A published loop with mods' slots (`[[native.loop]]`): its entry's copy runs the loop with them.
#[derive(Debug, Clone)]
pub struct LoopSlots {
    pub published: crate::loops::PublishedLoop,
    /// The slots, in the order they are asked: `(priority, mod id)`.
    pub slots: Vec<LoopSlot>,
}

/// One mod's slot in a published loop: its helper's items, compiled in the loop's module like a
/// mixin helper, and the slot type they define.
#[derive(Debug, Clone)]
pub struct LoopSlot {
    /// The mod's `[mod] id`.
    pub mod_id: String,
    /// The row's id.
    pub id: String,
    /// The helper's text.
    pub helper: String,
    /// The slot type the helper defines.
    pub with: String,
}

/// What a mod set adds to the patch crate beside its copies.
#[derive(Debug, Clone, Default)]
pub struct Extras {
    pub modules: Vec<SupportModule>,
    pub bridges: Vec<Bridge>,
    /// The loops whose entries are among the targets, with their slots.
    pub loops: Vec<LoopSlots>,
    /// The identifiers the rows' helpers and bindings use: a lower crate's mirror keeps a module's
    /// `use` only if a copied body or one of these names it.
    pub idents: BTreeSet<String>,
}

/// Why a target can't be copied out: a named refusal, never a game rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub target: String,
    pub code: &'static str,
    pub why: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}: {}", self.code, self.target, self.why)
    }
}

/// Macros whose expansion depends on the file or crate they are written in.
const PLACE_MACROS: &[&str] = &[
    "file",
    "line",
    "column",
    "module_path",
    "include",
    "include_str",
    "include_bytes",
    "env",
    "option_env",
];

/// One source module: its items, from its file or its inline `mod` block.
pub(crate) struct Module {
    pub(crate) items: Vec<Item>,
}

/// The game crate's modules by path, parsed lazily from the pinned source.
pub(crate) struct Source {
    root: PathBuf,
    /// The crates whose modules are in `files`.
    loaded: BTreeSet<String>,
    pub(crate) files: BTreeMap<String, Vec<PathBuf>>,
    parsed: BTreeMap<String, Module>,
    /// `macro_rules!` names the crate defines without `#[macro_export]`: a body using one can't
    /// leave the crate.
    local_macros: BTreeSet<String>,
    /// Each loaded lower crate's library name (`[lib] name`), by its folder's crate name.
    pub(crate) libraries: BTreeMap<String, String>,
    /// Each crate's declared types, read when a row first targets a trait method of the crate.
    owners: BTreeMap<String, crate::replace::OwnerTypes>,
    /// What [`replaceable`] reads once for every function it is asked about.
    cache: Cache,
    /// Each file's SHA-256 in the function index: a file that differs is refused as it is read.
    hashes: BTreeMap<PathBuf, String>,
    /// Why a file read did not match the index, when one did not: the build is refused, even where
    /// the read's failure was taken as "no such module".
    stale: Option<String>,
}

/// The walks [`replaceable`] would otherwise repeat per function: the index asks it about every
/// function of the game.
#[derive(Default)]
struct Cache {
    /// The traits the game's crates declare.
    traits: Option<BTreeSet<String>>,
    /// The macro names the modules on each path define.
    on_path: BTreeMap<String, BTreeSet<String>>,
}

impl Source {
    /// The modules the function index lists, with nothing walked: each file is read when a
    /// target's module is.
    fn indexed(root: &Path, index: &crate::functions::FunctionIndex) -> Result<Self, String> {
        let mut files = BTreeMap::new();
        let mut libraries = BTreeMap::new();
        let mut hashes = BTreeMap::new();
        for (krate, facts) in &index.crates {
            for (module, listed) in &facts.modules {
                let mut joined = Vec::new();
                for file in listed {
                    let path = crate::functions::source_path(root, &file.path)?;
                    hashes.insert(path.clone(), file.sha256.clone());
                    joined.push(path);
                }
                files.insert(module.clone(), joined);
            }
            libraries.insert(krate.clone(), facts.library.clone());
        }
        Ok(Self {
            root: root.to_owned(),
            loaded: index.crates.keys().cloned().collect(),
            files,
            parsed: BTreeMap::new(),
            local_macros: BTreeSet::new(),
            libraries,
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes,
            stale: None,
        })
    }

    /// The source root.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The text of `path`, refused when the function index has another hash for it.
    pub(crate) fn read(&mut self, path: &Path) -> Result<String, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some(expected) = self.hashes.get(path)
            && crate::functions::sha256(text.as_bytes()) != *expected
        {
            let why = format!(
                "{} differs from this game build's {}: the source is not the build's",
                path.display(),
                crate::functions::FILE
            );
            self.stale.get_or_insert_with(|| why.clone());
            return Err(why);
        }
        Ok(text)
    }

    pub(crate) fn new(root: &Path) -> Result<Self, String> {
        let src = root.join("crates/horizon-client/src");
        let mut files = BTreeMap::new();
        files.insert("horizon_client".to_owned(), vec![src.join("lib.rs")]);
        for file in crate::source::game_files(&src, false)? {
            files
                .entry(file.module)
                .or_insert_with(Vec::new)
                .push(file.path);
        }
        let mut local_macros = BTreeSet::new();
        let mut exported = BTreeSet::new();
        for path in walk(&src) {
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            macro_names(&text, &mut local_macros, &mut exported);
        }
        local_macros.retain(|name| !exported.contains(name));
        Ok(Self {
            root: root.to_owned(),
            loaded: BTreeSet::from(["horizon_client".to_owned()]),
            files,
            parsed: BTreeMap::new(),
            local_macros,
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        })
    }

    /// Adds a lower crate's modules, once: `false` when the source has no such crate.
    pub(crate) fn load(&mut self, krate: &str) -> bool {
        if self.loaded.contains(krate) {
            return true;
        }
        let Ok((package, lib)) = crate::source::crate_root(&self.root, krate) else {
            return false;
        };
        let Ok(library) = crate::replace::library_name(&package) else {
            return false;
        };
        let dir = lib.parent().expect("a source file has a folder").to_owned();
        let Ok(files) = crate::source::module_files(krate.to_owned(), lib.clone(), dir, false)
        else {
            return false;
        };
        let mut exported = BTreeSet::new();
        let mut local = BTreeSet::new();
        for file in files {
            let text = std::fs::read_to_string(&file.path).unwrap_or_default();
            macro_names(&text, &mut local, &mut exported);
            self.files.entry(file.module).or_default().push(file.path);
        }
        self.local_macros
            .extend(local.into_iter().filter(|name| !exported.contains(name)));
        self.libraries.insert(krate.to_owned(), library);
        self.loaded.insert(krate.to_owned());
        true
    }

    /// The types `krate` declares, from the files the staging pass reads: the installer a
    /// trait method has in the prebuilt game was decided on the same set.
    fn owner_types(&mut self, krate: &str) -> Result<&crate::replace::OwnerTypes, String> {
        if !self.owners.contains_key(krate) {
            let files = if krate == "horizon_client" {
                crate::source::game_files(&self.root.join("crates/horizon-client/src"), true)?
            } else {
                crate::source::crate_files(&self.root, krate)?
            };
            let mut types = crate::replace::OwnerTypes::default();
            for file in files {
                let text = std::fs::read_to_string(&file.path)
                    .map_err(|e| format!("{}: {e}", file.path.display()))?;
                let parsed =
                    syn::parse_file(&text).map_err(|e| format!("{}: {e}", file.path.display()))?;
                types.declare(&file.module, &parsed.items);
            }
            self.owners.insert(krate.to_owned(), types);
        }
        Ok(&self.owners[krate])
    }

    /// Module `path`'s items: its file's, or an inline module's inside its parent's.
    pub(crate) fn module(&mut self, path: &str) -> Result<&Module, String> {
        if !self.parsed.contains_key(path) {
            let items = if let Some(files) = self.files.get(path).cloned() {
                let mut items = Vec::new();
                for file in &files {
                    let text = self.read(file)?;
                    items.extend(
                        syn::parse_file(&text)
                            .map_err(|e| format!("{}: {e}", file.display()))?
                            .items,
                    );
                }
                items
            } else {
                let (parent, name) = path
                    .rsplit_once("::")
                    .ok_or_else(|| format!("no module {path}"))?;
                let parent = self.module(parent)?;
                parent
                    .items
                    .iter()
                    .find_map(|item| match item {
                        Item::Mod(module) if module.ident == name => {
                            module.content.as_ref().map(|(_, items)| items.clone())
                        }
                        _ => None,
                    })
                    .ok_or_else(|| format!("no module {path}"))?
            };
            self.parsed.insert(path.to_owned(), Module { items });
        }
        Ok(&self.parsed[path])
    }

    /// The `macro_rules!` items, not exported, that the modules on `path` define, outermost
    /// first: the macros a function in `path` may use by their bare names.
    fn path_macros(&mut self, path: &str) -> Vec<syn::ItemMacro> {
        let segments: Vec<&str> = path.split("::").collect();
        let mut out = Vec::new();
        for end in 1..=segments.len() {
            out.extend(self.scope_macros(&segments[..end].join("::")));
        }
        out
    }

    /// The macros module `path` itself defines and those its `#[macro_use] mod` children do,
    /// which textual scope carries on to the rest of `path`.
    fn scope_macros(&mut self, path: &str) -> Vec<syn::ItemMacro> {
        let Ok(module) = self.module(path) else {
            return Vec::new();
        };
        let items = module.items.clone();
        let mut out = module_macros(&items);
        for item in &items {
            if let Item::Mod(child) = item
                && child.attrs.iter().any(|a| a.path().is_ident("macro_use"))
            {
                out.extend(self.scope_macros(&format!("{path}::{}", child.ident)));
            }
        }
        out
    }

    /// Whether `path` names a module of a loaded crate.
    fn is_module(&mut self, path: &str) -> bool {
        self.files.contains_key(path) || self.module(path).is_ok()
    }
}

/// The names `text` defines with `macro_rules!`, into `local` or (with `#[macro_export]`)
/// `exported`.
fn macro_names(text: &str, local: &mut BTreeSet<String>, exported: &mut BTreeSet<String>) {
    for (at, _) in text.match_indices("macro_rules!") {
        let name: String = text[at + "macro_rules!".len()..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if text[..at].trim_end().ends_with("#[macro_export]") {
            exported.insert(name);
        } else {
            local.insert(name);
        }
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out
}

/// A type or path without its generic arguments or spaces: `Game<'_>` is `Game`.
pub(crate) fn plain(text: &str) -> String {
    let mut depth = 0usize;
    text.chars()
        .filter(|&c| match c {
            '<' => {
                depth += 1;
                false
            }
            '>' => {
                depth = depth.saturating_sub(1);
                false
            }
            c => depth == 0 && !c.is_whitespace(),
        })
        .collect()
}

/// What a copied body may not contain.
#[derive(Default)]
struct BodyCheck<'a> {
    local_macros: Option<&'a BTreeSet<String>>,
    refusal: Option<String>,
}

impl<'ast> Visit<'ast> for BodyCheck<'_> {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Some(last) = mac.path.segments.last() {
            let name = last.ident.to_string();
            if mac.path.segments.len() == 1 && PLACE_MACROS.contains(&name.as_str()) {
                self.refusal.get_or_insert(format!(
                    "its body uses `{name}!`, which means something else outside the game's source"
                ));
            } else if mac.path.segments.len() == 1 && name == "thread_local" {
                self.refusal.get_or_insert(
                    "its body declares a thread-local: a copy would have a second one".into(),
                );
            } else if mac.path.segments.len() == 1
                && self.local_macros.is_some_and(|local| local.contains(&name))
            {
                self.refusal.get_or_insert(format!(
                    "its body uses `{name}!`, a macro defined outside its module's path"
                ));
            }
        }
        syn::visit::visit_macro(self, mac);
    }
    fn visit_item(&mut self, item: &'ast Item) {
        match item {
            Item::Static(_) => {
                self.refusal.get_or_insert(
                    "its body declares a `static`: a copy would have a second one".into(),
                );
            }
            Item::Impl(_) => {
                self.refusal.get_or_insert(
                    "its body implements a trait or type: a copy would implement it twice".into(),
                );
            }
            Item::Macro(mac) if mac.ident.is_some() => {
                self.refusal.get_or_insert(
                    "its body defines a `macro_rules!`, which moves with different hygiene".into(),
                );
            }
            _ => {}
        }
        syn::visit::visit_item(self, item);
    }
}

/// The function a target names, found in its module.
pub(crate) struct Found {
    pub(crate) module: String,
    /// The `impl` it is in: none for a free function. Only its header is read.
    pub(crate) owner: Option<syn::ItemImpl>,
    pub(crate) function: syn::ImplItemFn,
}

/// A target's path as `[[native.mixin]]` rows and bridges write it: what comes before its last
/// segment, its name, and for a trait method (`module::<Type as Trait>::name`) the module, the
/// type without generic arguments or spaces, and the trait's last segment. The one parse of a
/// target: [`find`] and the function index's keys both read it.
pub(crate) struct TargetPath<'a> {
    pub(crate) front: &'a str,
    pub(crate) name: &'a str,
    pub(crate) qualified: Option<(&'a str, String, String)>,
}

pub(crate) fn target_path(target: &str) -> Option<TargetPath<'_>> {
    let (front, name) = target.rsplit_once("::")?;
    let qualified = front.split_once("::<").and_then(|(module, rest)| {
        let (ty, tr) = rest.strip_suffix('>')?.split_once(" as ")?;
        let tr = plain(tr);
        let last = tr.rsplit("::").next().unwrap_or_default().to_owned();
        Some((module, plain(ty), last))
    });
    Some(TargetPath {
        front,
        name,
        qualified,
    })
}

/// The function a bridge's target names, looked up by name in its module. A mixin's target is
/// read at its indexed span instead ([`crate::functions::FunctionIndex::found`]).
pub(crate) fn find(source: &mut Source, target: &str) -> Result<Found, (&'static str, String)> {
    let TargetPath {
        front,
        name,
        qualified,
    } = target_path(target).ok_or(("E_MIXIN_TARGET_MISSING", "not a function path".to_owned()))?;
    let krate = front.split("::").next().unwrap_or_default();
    if !source.load(krate) {
        return Err((
            "E_MIXIN_TARGET_MISSING",
            format!("no crate `{krate}` in the game's source"),
        ));
    }
    // A trait method is written `module::<Type as Trait>::name`.
    let (module, owner, trait_name) = match qualified {
        Some((module, ty, last)) => (module.to_owned(), ty, Some(last)),
        None => {
            // The longest prefix that is a module; what follows it is the `impl`'s type.
            let segments: Vec<&str> = front.split("::").collect();
            let mut cut = segments.len();
            while cut > 0 && !source.is_module(&segments[..cut].join("::")) {
                cut -= 1;
            }
            (
                segments[..cut].join("::"),
                plain(&segments[cut..].join("::")),
                None,
            )
        }
    };
    let items = source
        .module(&module)
        .map_err(|why| ("E_MIXIN_TARGET_MISSING", why))?
        .items
        .clone();
    let mut found = Vec::new();
    for item in &items {
        match item {
            Item::Fn(function) if owner.is_empty() && function.sig.ident == name => {
                found.push(Found {
                    module: module.clone(),
                    owner: None,
                    function: syn::ImplItemFn {
                        attrs: function.attrs.clone(),
                        vis: function.vis.clone(),
                        defaultness: None,
                        sig: function.sig.clone(),
                        block: (*function.block).clone(),
                    },
                })
            }
            Item::Impl(block)
                if !owner.is_empty()
                    && block.trait_.as_ref().map(|(_, path, _)| {
                        path.segments
                            .last()
                            .map(|s| s.ident.to_string())
                            .unwrap_or_default()
                    }) == trait_name
                    && plain(&block.self_ty.to_token_stream().to_string()) == owner =>
            {
                for member in &block.items {
                    if let ImplItem::Fn(function) = member
                        && function.sig.ident == name
                    {
                        found.push(Found {
                            module: module.clone(),
                            owner: Some(block.clone()),
                            function: function.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    match found.len() {
        0 => Err((
            "E_MIXIN_TARGET_MISSING",
            format!(
                "no function `{name}` in {module}{}",
                if owner.is_empty() {
                    String::new()
                } else {
                    format!(" on {owner}")
                }
            ),
        )),
        1 => Ok(found.pop().expect("one")),
        _ => Err((
            "E_MIXIN_TARGET_AMBIGUOUS",
            "several functions have this path (a cfg-gated pair, for one)".into(),
        )),
    }
}

/// Whether `function` can be copied and installed through its function's replacement slot: the
/// verdict the function index records for it.
pub(crate) fn replaceable(
    found: &Found,
    source: &mut Source,
    loop_generic: bool,
) -> Result<(), String> {
    // A crate-local macro defined in a module on the function's path moves with the copy (the
    // mirror repeats it); one defined elsewhere would not resolve.
    if !source.cache.on_path.contains_key(&found.module) {
        let names = source
            .path_macros(&found.module)
            .iter()
            .filter_map(|mac| mac.ident.as_ref().map(ToString::to_string))
            .collect();
        source.cache.on_path.insert(found.module.clone(), names);
    }
    let local_macros: BTreeSet<String> = source
        .local_macros
        .difference(&source.cache.on_path[&found.module])
        .cloned()
        .collect();
    let local_macros = &local_macros;
    let sig = &found.function.sig;
    // A published loop's function is generic over its slot alone: each mod build copies it once,
    // for the one slot type its entry's copy calls it with. Its other rules are a function's.
    let mut unslotted = sig.clone();
    if loop_generic {
        unslotted.generics = syn::Generics::default();
    }
    if !crate::replace::eligible(&found.function.attrs, &unslotted) {
        return Err("it is not replaceable: `const`, `async`, `unsafe`, \
             `self` by value, `impl Trait` or `!` in its signature, `#[inline(always)]`, or \
             compiled only under a `cfg`"
            .into());
    }
    if !loop_generic
        && !(crate::replace::lifetimes_only(&sig.generics)
            && found
                .owner
                .as_ref()
                .is_none_or(|b| crate::replace::lifetimes_only(&b.generics)))
    {
        return Err("it is generic over types or takes `impl Trait`: one copy can't stand for every instance".into());
    }
    if let Some(block) = &found.owner
        && block.trait_.is_some()
    {
        struct Associated(bool);
        impl<'ast> Visit<'ast> for Associated {
            fn visit_path(&mut self, path: &'ast syn::Path) {
                self.0 |= path.segments.len() > 1 && path.segments[0].ident == "Self";
                syn::visit::visit_path(self, path);
            }
        }
        let mut associated = Associated(false);
        let (_, path, _) = block.trait_.as_ref().expect("a trait implementation");
        associated.visit_signature(&crate::replace::qualified(sig, path));
        if associated.0 {
            return Err("its signature names a trait-associated constant through Self".into());
        }
        let foreign = crate::replace::foreign_imports(&source.module(&found.module)?.items);
        let krate = found
            .module
            .split("::")
            .next()
            .unwrap_or_default()
            .to_owned();
        if source.cache.traits.is_none() {
            source.cache.traits = Some(crate::replace::declared_traits(&source.root)?);
        }
        source.owner_types(&krate)?;
        let traits = source.cache.traits.as_ref().expect("read above");
        if let Some(why) = crate::replace::trait_refusal(
            &found.module,
            block,
            traits,
            &source.owners[&krate],
            &foreign,
        ) {
            return Err(why);
        }
    }
    if let Some(block) = &found.owner
        && block.attrs.iter().any(|attr| attr.path().is_ident("cfg"))
    {
        return Err("its `impl` is compiled only under a `cfg`".into());
    }
    let mut check = BodyCheck {
        local_macros: Some(local_macros),
        refusal: None,
    };
    check.visit_block(&found.function.block);
    match check.refusal {
        Some(why) => Err(why),
        None => Ok(()),
    }
}

/// The names a module's own items define, for explicit re-export (an explicit name outranks the
/// module's glob imports, as the item itself did).
fn defined(items: &[Item]) -> Vec<(syn::Ident, Vec<syn::Attribute>)> {
    let cfg = |attrs: &[syn::Attribute]| {
        attrs
            .iter()
            .filter(|attr| attr.path().is_ident("cfg"))
            .cloned()
            .collect::<Vec<_>>()
    };
    // Test-only items are not in the prebuilt game.
    let test_only = |attrs: &[syn::Attribute]| {
        attrs.iter().any(|attr| {
            attr.path().is_ident("test")
                || (attr.path().is_ident("cfg")
                    && attr.to_token_stream().to_string().contains("test"))
        })
    };
    items
        .iter()
        .filter(|item| {
            let attrs = match item {
                Item::Fn(item) => &item.attrs,
                Item::Mod(item) => &item.attrs,
                Item::Struct(item) => &item.attrs,
                Item::Enum(item) => &item.attrs,
                Item::Const(item) => &item.attrs,
                Item::Static(item) => &item.attrs,
                Item::Type(item) => &item.attrs,
                Item::Trait(item) => &item.attrs,
                Item::Union(item) => &item.attrs,
                _ => return true,
            };
            !test_only(attrs)
        })
        .filter_map(|item| match item {
            Item::Fn(item) => Some((item.sig.ident.clone(), cfg(&item.attrs))),
            Item::Struct(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Enum(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Union(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Const(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Static(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Type(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Trait(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Mod(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            _ => None,
        })
        .filter(|(ident, attrs)| {
            ident != "tests"
                && !attrs
                    .iter()
                    .any(|a| a.to_token_stream().to_string().contains("test"))
        })
        .collect()
}

/// A module's own `macro_rules!` items that it does not export.
fn module_macros(items: &[Item]) -> Vec<syn::ItemMacro> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Macro(mac)
                if mac.ident.is_some()
                    && mac.mac.path.is_ident("macro_rules")
                    && !mac.attrs.iter().any(|a| a.path().is_ident("macro_export")) =>
            {
                Some(mac.clone())
            }
            _ => None,
        })
        .collect()
}

/// The module's `use` items, as written.
fn uses(items: &[Item]) -> Vec<&syn::ItemUse> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Use(item) => Some(item),
            _ => None,
        })
        .collect()
}

/// A copied function's attributes: its docs, lint levels and cfg, nothing that changes codegen
/// or the point machinery.
fn kept(attrs: &[syn::Attribute]) -> Vec<syn::Attribute> {
    attrs
        .iter()
        .filter(|attr| {
            ["doc", "allow", "cfg", "expect", "warn"]
                .iter()
                .any(|name| attr.path().is_ident(name))
        })
        .cloned()
        .collect()
}

/// A tree of mirror modules, keyed by segment.
#[derive(Default)]
struct Tree {
    children: BTreeMap<String, Tree>,
    copies: Vec<proc_macro2::TokenStream>,
    /// Names the copies define, which the mirror must not also re-export.
    own: BTreeSet<String>,
}

/// The patch crate for `targets` (as `[[native.mixin]]` rows write them), from the pinned game source
/// at `root`, which `index` (the build's `functions.json`) describes. `krate` is the patch crate's
/// name. A target's file and verdict come from the index, and its text is checked against it
/// before it is copied: the source is never walked. Every target that can't be copied is refused
/// by name; the crate is returned only when none is.
pub fn generate(
    root: &Path,
    krate: &str,
    targets: &[String],
    extras: &Extras,
    index: &crate::functions::FunctionIndex,
) -> Result<PatchCrate, Vec<Refusal>> {
    let refuse = |target: &str, code, why: String| Refusal {
        target: target.to_owned(),
        code,
        why,
    };
    let mut source = match Source::indexed(root, index) {
        Ok(source) => source,
        Err(why) => {
            return Err(targets
                .iter()
                .map(|t| refuse(t, "E_MIXIN_SOURCE", why.clone()))
                .collect());
        }
    };
    let mut refusals = Vec::new();
    let mut trees: BTreeMap<String, Tree> = BTreeMap::new();
    let mut patched = Vec::new();
    let unique: BTreeSet<&String> = targets.iter().collect();
    // The loops whose generic function a row targets: their entries' copies call the copy.
    let generic_copied: BTreeSet<&str> = extras
        .loops
        .iter()
        .filter(|l| unique.contains(&l.published.generic_target()))
        .map(|l| l.published.name.as_str())
        .collect();
    for target in unique {
        let loop_generic = extras
            .loops
            .iter()
            .any(|l| l.published.generic_target() == *target);
        if let Err((code, why)) = index.check(root, target) {
            refusals.push(refuse(target, code, why));
            continue;
        }
        let found = match index.found(&mut source, target) {
            Ok(found) => found,
            Err((code, why)) => {
                refusals.push(refuse(target, code, why));
                continue;
            }
        };
        let name = found.function.sig.ident.clone();
        let source_crate = found.module.split("::").next().expect("a crate").to_owned();
        let patch_name = patch_crate(krate, &source_crate);
        let mut node = trees.entry(source_crate).or_default();
        for segment in found.module.split("::").skip(1) {
            node = node.children.entry(segment.to_owned()).or_default();
        }
        let attrs = kept(&found.function.attrs);
        let mut sig = found.function.sig.clone();
        // A loop's function is copied for the one slot type its entry's copy runs it with:
        // horizon-mixins patches only non-generic functions.
        let filled_block = extras
            .loops
            .iter()
            .find(|l| loop_generic && l.published.generic_target() == *target)
            .map(|slots| fill_slot(&mut sig, &found.function.block, slots));
        let entry_of = extras.loops.iter().find(|l| l.published.entry == *target);
        let loop_block = match entry_of
            .map(|slots| {
                loop_entry(
                    slots,
                    generic_copied.contains(slots.published.name.as_str()),
                )
            })
            .transpose()
        {
            Ok(entry) => entry.map(|(items, block)| {
                node.copies.push(items);
                block
            }),
            Err(refusal) => {
                refusals.push(refuse(target, "E_LOOP_HELPER", refusal));
                continue;
            }
        };
        let block = loop_block
            .as_ref()
            .or(filled_block.as_ref())
            .unwrap_or(&found.function.block);
        let install = format_ident!("__fnr_install_{}", name);
        let constructor = format_ident!("__fnr_{}", name);
        let module_path = found.module.split("::").skip(1).collect::<Vec<_>>();
        let (copy, id, install) = match &found.owner {
            None => {
                node.own.insert(name.to_string());
                node.own.insert(install.to_string());
                // The extern a lower crate is named by is its library's name (`mod_render`), not
                // its folder's (`horizon_mod_render`).
                let original: syn::Path =
                    syn::parse_str(&original_path(&source.libraries, &found.module))
                        .expect("a module path");
                (
                    quote! {
                        #(#attrs)*
                        pub #sig #block

                        #[doc(hidden)]
                        pub fn #install() -> bool {
                            #original::#constructor(#name)
                        }
                    },
                    format!("{patch_name}::{}::{name}", module_path.join("::")),
                    install,
                )
            }
            Some(owner) => {
                let self_ty = &owner.self_ty;
                let owner_name = plain(&self_ty.to_token_stream().to_string()).replace("::", "_");
                // A trait method's copy is renamed in its helper trait: with the game's trait in
                // scope too, `x.name()` would be ambiguous.
                let source_trait = owner.trait_.as_ref().map(|(_, path, _)| path.clone());
                let source_trait_name = source_trait
                    .as_ref()
                    .and_then(|path| path.segments.last())
                    .map(|segment| segment.ident.to_string());
                let (trait_name, install, method, constructor) = match &source_trait_name {
                    Some(tr) => (
                        format_ident!("__Fnr_{}_as_{}_{}", owner_name, tr, name),
                        format_ident!("__fnr_install_{}_as_{}_{}", owner_name, tr, name),
                        format_ident!("__fnr_{}", name),
                        format_ident!("__fnr_as_{}_{}", tr, name),
                    ),
                    None => (
                        format_ident!("__Fnr_{}_{}", owner_name, name),
                        format_ident!("__fnr_install_{}_{}", owner_name, name),
                        name.clone(),
                        constructor.clone(),
                    ),
                };
                sig.ident = method.clone();
                let mut block = block.clone();
                if let Some(path) = &source_trait {
                    qualify_self(&mut block, owner, self_ty, path);
                    qualify_self_sig(&mut sig, owner, self_ty, path);
                }
                node.own.insert(trait_name.to_string());
                node.own.insert(install.to_string());
                let generics = &owner.generics;
                let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
                // A trait declaration takes no patterns: its parameters are renamed.
                let mut declared = sig.clone();
                for (index, input) in declared.inputs.iter_mut().enumerate() {
                    if let syn::FnArg::Typed(typed) = input {
                        let fresh = format_ident!("__a{index}");
                        *typed.pat = syn::parse_quote!(#fresh);
                    }
                }
                sig.constness = None;
                // `Game<'_>` and `Game<'g>` alike are named with elided lifetimes in expressions.
                let mut elided = (**self_ty).clone();
                struct Elide;
                impl syn::visit_mut::VisitMut for Elide {
                    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
                        if lifetime.ident != "static" {
                            *lifetime = syn::parse_quote!('_);
                        }
                    }
                }
                syn::visit_mut::VisitMut::visit_type_mut(&mut Elide, &mut elided);
                let trait_args: Vec<_> = generics.lifetimes().map(|_| quote!('_)).collect();
                let installer = (!loop_generic).then(|| {
                    let trait_ref = if trait_args.is_empty() {
                        quote!(#trait_name)
                    } else {
                        quote!(#trait_name<#(#trait_args),*>)
                    };
                    quote! {
                        #[doc(hidden)]
                        pub fn #install() -> bool {
                            <#elided>::#constructor(<#elided as #trait_ref>::#method)
                        }
                    }
                });
                let id_type = plain_spaces(&elided.to_token_stream().to_string());
                let id_trait = plain_spaces(&{
                    let generics_args: Vec<_> =
                        generics.lifetimes().map(|l| l.lifetime.clone()).collect();
                    if generics_args.is_empty() {
                        quote!(#trait_name).to_string()
                    } else {
                        quote!(#trait_name<#(#generics_args),*>).to_string()
                    }
                });
                (
                    quote! {
                        #[doc(hidden)]
                        #[allow(non_camel_case_types)]
                        pub trait #trait_name #impl_generics #where_clause {
                            #declared;
                        }
                        impl #impl_generics #trait_name #type_generics for #self_ty #where_clause {
                            #(#attrs)*
                            #sig #block
                        }

                        #installer
                    },
                    format!(
                        "{patch_name}::{}::<{id_type} as {id_trait}>::{method}",
                        module_path.join("::")
                    ),
                    install,
                )
            }
        };
        node.copies.push(copy);
        patched.push(Patched {
            target: target.clone(),
            id,
            install: (!loop_generic)
                .then(|| format!("{patch_name}::{}::{install}()", module_path.join("::"))),
        });
    }
    for module in &extras.modules {
        match support_module(&source, module) {
            Ok((path, tokens)) => {
                let mut node = trees.entry("horizon_client".to_owned()).or_default();
                for segment in path.split("::").skip(1) {
                    node = node.children.entry(segment.to_owned()).or_default();
                }
                node.own.insert(module.name.clone());
                node.copies.push(tokens);
            }
            Err((code, why)) => refusals.push(refuse(&module.id, code, why)),
        }
    }
    let mut bridge_traits = Vec::new();
    for bridge in &extras.bridges {
        match bridge_trait(&mut source, bridge) {
            Ok((path, trait_name, tokens)) => {
                let mut node = trees.entry("horizon_client".to_owned()).or_default();
                for segment in path.split("::").skip(1) {
                    node = node.children.entry(segment.to_owned()).or_default();
                }
                node.own.insert(trait_name.to_string());
                node.copies.push(tokens);
                let mirror: syn::Path =
                    syn::parse_str(&path.replacen("horizon_client", "crate", 1))
                        .expect("a module path");
                bridge_traits.push(quote!(pub use #mirror::#trait_name;));
            }
            Err((code, why)) => refusals.push(refuse(&bridge.id, code, why)),
        }
    }
    if let Some(why) = source.stale.take() {
        return Err(targets
            .iter()
            .map(|t| refuse(t, "E_MIXIN_SOURCE", why.clone()))
            .collect());
    }
    if !refusals.is_empty() {
        return Err(refusals);
    }
    let mut libs = Vec::new();
    for (source_crate, tree) in &trees {
        let bridges = source_crate == "horizon_client" && !bridge_traits.is_empty();
        let used = (source_crate != "horizon_client").then(|| {
            let mut used = extras.idents.clone();
            idents_in(&tree_tokens(tree), &mut used);
            used
        });
        let body = match mirror(&mut source, source_crate, tree, bridges, used.as_ref()) {
            Ok(body) => body,
            Err(why) => {
                return Err(targets
                    .iter()
                    .map(|t| refuse(t, "E_MIXIN_SOURCE", why.clone()))
                    .collect());
            }
        };
        let bridge_module = bridges.then(|| {
            quote! {
                /// The bridges' traits, which every mirror module imports.
                #[doc(hidden)]
                pub mod __fnr_bridges {
                    #(#bridge_traits)*
                }
            }
        });
        let lib = quote! {
            //! Generated by the launcher: game functions copied out of the pinned source for the
            //! installed mods' `[[native.mixin]]` rows, patched by horizon-mixins. Not for editing.
            #![allow(unused_imports, unused_macros, unused_mut, unused_variables, dead_code, unused_braces, unused_parens)]
            #![allow(clippy::all)]
            #body
            #bridge_module
        };
        libs.push((
            patch_crate(krate, source_crate),
            prettyplease_or_tokens(lib),
        ));
    }
    // A mirror's module read from a file that is not the build's.
    if let Some(why) = source.stale.take() {
        return Err(targets
            .iter()
            .map(|t| refuse(t, "E_MIXIN_SOURCE", why.clone()))
            .collect());
    }
    Ok(PatchCrate { libs, patched })
}

/// The loop function's signature `sig` with its slot type parameter replaced by the slots `slots`
/// fills it with (`()` with none), and its body `block` with the same replacement: the type in
/// type position, and `<Slots>::NAME` for a path through it (`S::RECHECK`).
fn fill_slot(sig: &mut syn::Signature, block: &syn::Block, slots: &LoopSlots) -> syn::Block {
    let mut block = block.clone();
    let Some(param) = sig.generics.type_params().next().map(|p| p.ident.clone()) else {
        return block;
    };
    sig.generics.params = sig
        .generics
        .params
        .iter()
        .filter(|p| !matches!(p, syn::GenericParam::Type(_)))
        .cloned()
        .collect();
    let filled: syn::Type = if slots.slots.is_empty() {
        syn::parse_quote!(())
    } else {
        let ty = format_ident!("__FnrLoopSlots_{}", loop_tag(&slots.published.name));
        syn::parse_quote!(#ty)
    };
    struct Fill(syn::Ident, syn::Type);
    impl Fill {
        /// `S::rest` as `<Slots>::rest`; `None` for any other path.
        fn through(&self, qself: &Option<syn::QSelf>, path: &syn::Path) -> Option<syn::TypePath> {
            let first = path.segments.first()?;
            if qself.is_some()
                || path.leading_colon.is_some()
                || path.segments.len() < 2
                || first.ident != self.0
                || !first.arguments.is_none()
            {
                return None;
            }
            let ty = &self.1;
            let rest = path.segments.iter().skip(1);
            Some(syn::parse_quote!(<#ty>::#(#rest)::*))
        }
    }
    impl syn::visit_mut::VisitMut for Fill {
        fn visit_type_mut(&mut self, ty: &mut syn::Type) {
            if matches!(ty, syn::Type::Path(p) if p.qself.is_none() && p.path.is_ident(&self.0)) {
                *ty = self.1.clone();
                return;
            }
            if let syn::Type::Path(p) = ty
                && let Some(filled) = self.through(&p.qself, &p.path)
            {
                *p = filled;
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
        fn visit_expr_path_mut(&mut self, expr: &mut syn::ExprPath) {
            if let Some(filled) = self.through(&expr.qself, &expr.path) {
                expr.qself = filled.qself;
                expr.path = filled.path;
            }
            syn::visit_mut::visit_expr_path_mut(self, expr);
        }
    }
    let mut fill = Fill(param, filled);
    syn::visit_mut::VisitMut::visit_signature_mut(&mut fill, sig);
    syn::visit_mut::VisitMut::visit_block_mut(&mut fill, &mut block);
    block
}

/// A published loop's name as an identifier part (`block_entities_tick`).
fn loop_tag(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// A loop entry's copy: each slot's helper as a module beside it, marked with its mod for the crash
/// report, and its body, which runs the loop with the slots in this game's state for it
/// (`Game::run_published_loop`). Refused when a helper is not Rust items.
fn loop_entry(
    slots: &LoopSlots,
    generic_copied: bool,
) -> Result<(proc_macro2::TokenStream, syn::Block), String> {
    let published = &slots.published;
    let tag = loop_tag(&published.name);
    let hook = format!("loop {}", published.name);
    let loops = quote!(::horizon_client::game::published_loops);
    let mut items = Vec::new();
    let mut nested = quote!(());
    for (index, slot) in slots.slots.iter().enumerate().rev() {
        let helper = syn::parse_file(&slot.helper)
            .map_err(|e| format!("{}:{}: its helper: {e}", slot.mod_id, slot.id))?
            .items;
        let module = format_ident!("__fnr_loop_{}_{}", index, tag);
        let marker = format_ident!("__FnrLoopMod_{}_{}", index, tag);
        let with = syn::parse_str::<syn::Ident>(&slot.with)
            .map_err(|e| format!("{}:{}: with: {e}", slot.mod_id, slot.id))?;
        let id = &slot.mod_id;
        let part = format!("{}::{}", slot.mod_id, slot.with);
        items.push(quote! {
            /// A `[[native.loop]]` helper, compiled where the loop's code is.
            #[doc(hidden)]
            #[allow(non_snake_case)]
            pub mod #module {
                use super::*;
                #(#helper)*
            }
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            pub struct #marker;
            impl #loops::LoopMod for #marker {
                const CALL: ::mod_api::content::crash::ModCall = ::mod_api::content::crash::ModCall {
                    id: #id,
                    hook: #hook,
                    part: Some(#part),
                };
            }
        });
        nested = quote!((#loops::Attributed<#marker, #module::#with>, #nested));
    }
    let slots_ty = format_ident!("__FnrLoopSlots_{}", tag);
    items.push(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub type #slots_ty = #nested;
    });
    let name = &published.name;
    let generic = format_ident!("{}", published.generic);
    // With rows on the loop's function, its patched copy (a method of its helper trait, beside this
    // one, for these slots) runs the loop; the game's own otherwise, and always for the run without
    // the slots (nested, or after a panic), which the copy's slot type can't take.
    let call = |game: proc_macro2::TokenStream, slot: proc_macro2::TokenStream| {
        if generic_copied {
            // The entry's owner type: the segment before its name.
            let owner = published
                .entry
                .rsplit("::")
                .nth(1)
                .map(plain)
                .unwrap_or_default();
            let helper = format_ident!("__Fnr_{}_{}", owner, published.generic);
            quote!(#helper::#generic(#game, #slot))
        } else {
            quote!(#game.#generic(#slot))
        }
    };
    let block = if slots.slots.is_empty() {
        let run = call(quote!(self), quote!(&mut ()));
        syn::parse_quote!({ #run })
    } else {
        let filled = call(quote!(game), quote!(slots));
        let vanilla = quote!(game.#generic(&mut ()));
        syn::parse_quote!({
            self.run_published_loop(
                #name,
                |game, slots: &mut #slots_ty| #filled,
                |game| #vanilla,
            )
        })
    };
    Ok((quote!(#(#items)*), block))
}

/// Every copy's tokens in `tree` and its children.
fn tree_tokens(tree: &Tree) -> proc_macro2::TokenStream {
    let copies = &tree.copies;
    let children = tree.children.values().map(tree_tokens);
    quote!(#(#copies)* #(#children)*)
}

/// The identifiers in `tokens`, into `out`.
pub fn idents_in(tokens: &proc_macro2::TokenStream, out: &mut BTreeSet<String>) {
    for tree in tokens.clone() {
        match tree {
            proc_macro2::TokenTree::Ident(ident) => {
                out.insert(ident.to_string());
            }
            proc_macro2::TokenTree::Group(group) => idents_in(&group.stream(), out),
            _ => {}
        }
    }
}

/// The patch crate that mirrors `source_crate`: `krate` itself for the game crate, and
/// `krate_<crate>` for a lower one (`horizon_patch_client_lib` for `horizon_client_lib`).
pub fn patch_crate(krate: &str, source_crate: &str) -> String {
    match source_crate {
        "horizon_client" => krate.to_owned(),
        other => format!("{krate}_{}", other.trim_start_matches("horizon_")),
    }
}

/// `Self::Item` for an associated type or constant of the trait `path` that `owner` implements,
/// rewritten `<Type as Trait>::Item`: in the helper trait `Self` has no such item.
struct QualifySelf {
    items: BTreeSet<String>,
    qualified: proc_macro2::TokenStream,
}

impl QualifySelf {
    fn new(owner: &syn::ItemImpl, self_ty: &syn::Type, path: &syn::Path) -> Self {
        let items = owner
            .items
            .iter()
            .filter_map(|item| match item {
                ImplItem::Type(item) => Some(item.ident.to_string()),
                ImplItem::Const(item) => Some(item.ident.to_string()),
                _ => None,
            })
            .collect();
        Self {
            items,
            qualified: quote!(<#self_ty as #path>),
        }
    }
    fn rewrite(&self, qself: &mut Option<syn::QSelf>, path: &mut syn::Path) {
        if qself.is_none()
            && path.segments.len() == 2
            && path.segments[0].ident == "Self"
            && self.items.contains(&path.segments[1].ident.to_string())
        {
            let item = &path.segments[1];
            let qualified = &self.qualified;
            let rewritten: syn::ExprPath =
                syn::parse2(quote!(#qualified::#item)).expect("a qualified path");
            *qself = rewritten.qself;
            *path = rewritten.path;
        }
    }
}

impl syn::visit_mut::VisitMut for QualifySelf {
    fn visit_expr_path_mut(&mut self, expr: &mut syn::ExprPath) {
        self.rewrite(&mut expr.qself, &mut expr.path);
        syn::visit_mut::visit_expr_path_mut(self, expr);
    }
    fn visit_type_path_mut(&mut self, ty: &mut syn::TypePath) {
        self.rewrite(&mut ty.qself, &mut ty.path);
        syn::visit_mut::visit_type_path_mut(self, ty);
    }
}

fn qualify_self(
    block: &mut syn::Block,
    owner: &syn::ItemImpl,
    self_ty: &syn::Type,
    path: &syn::Path,
) {
    syn::visit_mut::VisitMut::visit_block_mut(&mut QualifySelf::new(owner, self_ty, path), block);
}

fn qualify_self_sig(
    sig: &mut syn::Signature,
    owner: &syn::ItemImpl,
    self_ty: &syn::Type,
    path: &syn::Path,
) {
    syn::visit_mut::VisitMut::visit_signature_mut(&mut QualifySelf::new(owner, self_ty, path), sig);
}

/// An invoker's signature moves into a trait of its own, where `Self::Item` names nothing: an
/// item of the trait `owner` implements is written `<Type as Trait>::Item`, and any other is
/// refused, naming it.
fn invoker_self_items(sig: &mut syn::Signature, owner: &syn::ItemImpl) -> Result<(), String> {
    if let Some((_, path, _)) = &owner.trait_ {
        qualify_self_sig(sig, owner, &owner.self_ty, path);
    }
    struct Left(Option<String>);
    impl<'ast> syn::visit::Visit<'ast> for Left {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            if self.0.is_none() && path.segments.len() > 1 && path.segments[0].ident == "Self" {
                self.0 = Some(path.to_token_stream().to_string().replace(' ', ""));
            }
            syn::visit::visit_path(self, path);
        }
    }
    let mut left = Left(None);
    syn::visit::Visit::visit_signature(&mut left, sig);
    match left.0 {
        Some(item) => Err(format!(
            "an invoker's signature names `{item}`, which a bridge cannot name; bridge a function whose signature spells the type out"
        )),
        None => Ok(()),
    }
}

/// A path's tokens without spaces, as horizon-mixins writes an id.
fn plain_spaces(text: &str) -> String {
    text.replace(' ', "")
}

fn prettyplease_or_tokens(tokens: proc_macro2::TokenStream) -> String {
    match syn::parse2::<syn::File>(tokens.clone()) {
        Ok(file) => prettyplease::unparse(&file),
        Err(_) => tokens.to_string(),
    }
}

/// The module `path` of the game's source as the patch crate names it: a lower crate's folder
/// name gives the path's first segment, and its library's name (`[lib] name`) the extern.
fn original_path(libraries: &BTreeMap<String, String>, path: &str) -> String {
    let (krate, rest) = path
        .split_once("::")
        .map_or((path, None), |(k, r)| (k, Some(r)));
    let library = libraries.get(krate).map_or(krate, String::as_str);
    match rest {
        Some(rest) => format!("::{library}::{rest}"),
        None => format!("::{library}"),
    }
}

/// The mirror of module `path` and its children in `tree`.
fn mirror(
    source: &mut Source,
    path: &str,
    tree: &Tree,
    bridges: bool,
    used: Option<&BTreeSet<String>>,
) -> Result<proc_macro2::TokenStream, String> {
    let items = source.module(path)?.items.clone();
    let original: syn::Path =
        syn::parse_str(&original_path(&source.libraries, path)).expect("a module path");
    let reserved: BTreeSet<String> = tree
        .children
        .keys()
        .cloned()
        .chain(tree.own.iter().cloned())
        .collect();
    // The crate root is not widened: only its public items are there to re-export.
    let public: BTreeSet<String> = items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.sig.ident.to_string())
            }
            Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Const(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Static(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Trait(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Mod(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Union(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            _ => None,
        })
        .collect();
    let root = !path.contains("::");
    let only_public =
        root || (path.split("::").next() != Some("horizon_client") && tree.copies.is_empty());
    let explicit = defined(&items)
        .into_iter()
        .filter(|(ident, _)| !reserved.contains(&ident.to_string()))
        .filter(|(ident, _)| !only_public || public.contains(&ident.to_string()))
        .map(|(ident, cfg)| quote!(#(#cfg)* pub use #original::#ident;));
    let uses = uses(&items).into_iter().filter(|item| {
        // A `use` naming a child mirror or a copy would collide with it.
        let mut names = Vec::new();
        collect_names(&item.tree, &mut names);
        // In a lower crate only the listed functions' modules are widened, so an import of a
        // private item elsewhere would not resolve: keep only what the copies and helpers name.
        let wanted = used
            .is_none_or(|used| names.is_empty() || names.iter().any(|name| used.contains(name)));
        wanted && !names.iter().any(|name| reserved.contains(name))
    });
    let mut children = Vec::new();
    for (name, child) in &tree.children {
        let ident = format_ident!("{name}");
        let inner = mirror(source, &format!("{path}::{name}"), child, bridges, used)?;
        children.push(quote!(pub mod #ident { #inner }));
    }
    // Repeated before the children, which see them by textual scope as the game's did.
    let macros = source.scope_macros(path);
    let copies = &tree.copies;
    let bridge_import = bridges.then(|| {
        quote!(
            use crate::__fnr_bridges::*;
        )
    });
    // The explicit re-exports name every item the module defines; the glob is there only for
    // items a macro invocation defines, which syn can't see. A glob would also make
    // horizon-mixins take `format!`/`vec!`/`matches!` as possibly shadowed.
    let glob = items
        .iter()
        .any(|item| matches!(item, Item::Macro(mac) if mac.ident.is_none()))
        .then(|| quote!(pub use #original::*;));
    Ok(quote! {
        #glob
        #bridge_import
        #(#macros)*
        #(#explicit)*
        #(#uses)*
        #(#children)*
        #(#copies)*
    })
}

/// The module path whose file is `file`, and the support module's tokens.
fn support_module(
    source: &Source,
    module: &SupportModule,
) -> Result<(String, proc_macro2::TokenStream), (&'static str, String)> {
    let path = source
        .files
        .iter()
        .find(|(_, paths)| paths.iter().any(|path| path.ends_with(&module.file)))
        .map(|(name, _)| name.clone())
        .ok_or((
            "E_MIXIN_MODULE_DESTINATION",
            format!(
                "{} is not a file of the game crate (`horizon_client`); a support module goes \
                 into one of its files",
                module.file
            ),
        ))?;
    let items = syn::parse_file(&module.text)
        .map_err(|e| ("E_MIXIN_MODULE_SOURCE", e.to_string()))?
        .items;
    let name = format_ident!("{}", module.name);
    let vis = if module.public {
        quote!(pub)
    } else {
        quote!(pub(crate))
    };
    Ok((path, quote!(#vis mod #name { #(#items)* })))
}

/// The bridge's owner module, trait name and the trait with its impl, emitted in the mirror of
/// the owner's module so its types are named as the game names them.
fn bridge_trait(
    source: &mut Source,
    bridge: &Bridge,
) -> Result<(String, syn::Ident, proc_macro2::TokenStream), (&'static str, String)> {
    let method = format_ident!("{}", bridge.method);
    let trait_name = format_ident!("__Fnr_Bridge_{}", bridge.method);
    if bridge.kind == "INVOKE" {
        let found = find(source, &bridge.target)?;
        let owner = found.owner.as_ref().ok_or((
            "E_MIXIN_BRIDGE_TARGET",
            "an invoker's target is a method or associated function of a type".to_owned(),
        ))?;
        if !owner.generics.params.is_empty() || !found.function.sig.generics.params.is_empty() {
            return Err((
                "E_MIXIN_BRIDGE_TARGET",
                "an invoker's type and method are not generic".into(),
            ));
        }
        let self_ty = &owner.self_ty;
        let original = &found.function.sig.ident;
        let mut args = Vec::new();
        let mut sig = found.function.sig.clone();
        for (index, input) in sig.inputs.iter_mut().enumerate() {
            match input {
                syn::FnArg::Receiver(_) => args.push(quote!(self)),
                syn::FnArg::Typed(typed) => {
                    let name = format_ident!("__a{index}");
                    *typed.pat = syn::parse_quote!(#name);
                    args.push(quote!(#name));
                }
            }
        }
        sig.ident = method.clone();
        invoker_self_items(&mut sig, owner).map_err(|why| ("E_MIXIN_BRIDGE_TARGET", why))?;
        let call = match &owner.trait_ {
            Some((_, path, _)) => quote!(<#self_ty as #path>::#original(#(#args),*)),
            None => quote!(<#self_ty>::#original(#(#args),*)),
        };
        let path = found.module.clone();
        return Ok((
            path,
            trait_name.clone(),
            quote! {
                #[doc(hidden)]
                #[allow(non_camel_case_types)]
                pub trait #trait_name { #sig; }
                impl #trait_name for #self_ty { #sig { #call } }
            },
        ));
    }
    let (front, owner) = bridge.target.rsplit_once("::").ok_or((
        "E_MIXIN_BRIDGE_TARGET",
        "a field bridge's target is a struct path".to_owned(),
    ))?;
    if !source.is_module(front) {
        return Err((
            "E_MIXIN_BRIDGE_TARGET",
            format!("no module {front} in the game crate"),
        ));
    }
    let items = source
        .module(front)
        .map_err(|why| ("E_MIXIN_BRIDGE_TARGET", why))?
        .items
        .clone();
    let item = items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == owner => Some(item.clone()),
            _ => None,
        })
        .ok_or((
            "E_MIXIN_BRIDGE_TARGET",
            format!("no struct {owner} in {front}"),
        ))?;
    if item.generics.type_params().next().is_some() || item.generics.const_params().next().is_some()
    {
        return Err((
            "E_MIXIN_BRIDGE_TARGET",
            format!("{owner} is generic over types"),
        ));
    }
    let member_name = bridge.member.as_deref().ok_or((
        "E_MIXIN_BRIDGE_TARGET",
        "a field bridge names its `member`".to_owned(),
    ))?;
    let field = match &item.fields {
        syn::Fields::Named(fields) => fields
            .named
            .iter()
            .find(|f| f.ident.as_ref().is_some_and(|i| i == member_name)),
        syn::Fields::Unnamed(fields) => member_name
            .parse::<usize>()
            .ok()
            .and_then(|i| fields.unnamed.get(i)),
        syn::Fields::Unit => None,
    }
    .ok_or((
        "E_MIXIN_BRIDGE_TARGET",
        format!("{owner} has no field {member_name}"),
    ))?;
    let ty = &field.ty;
    let member: syn::Member =
        syn::parse_str(member_name).map_err(|e| ("E_MIXIN_BRIDGE_TARGET", e.to_string()))?;
    let owner = &item.ident;
    // A type generic over lifetimes gets a trait over the same lifetimes.
    let (generics, args, _) = item.generics.split_for_impl();
    let (declared, defined) = match bridge.kind.as_str() {
        "GET" => (quote!(fn #method(&self) -> &#ty), quote!({ &self.#member })),
        "GET_MUT" => (
            quote!(fn #method(&mut self) -> &mut #ty),
            quote!({ &mut self.#member }),
        ),
        "SET" => (
            quote!(fn #method(&mut self, value: #ty)),
            quote!({ self.#member = value; }),
        ),
        other => {
            return Err((
                "E_MIXIN_BRIDGE_TARGET",
                format!("unknown bridge kind {other}"),
            ));
        }
    };
    Ok((
        front.to_owned(),
        trait_name.clone(),
        quote! {
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            pub trait #trait_name #generics { #declared; }
            impl #generics #trait_name #args for #owner #args { #declared #defined }
        },
    ))
}

fn collect_names(tree: &syn::UseTree, out: &mut Vec<String>) {
    match tree {
        syn::UseTree::Path(path) => collect_names(&path.tree, out),
        syn::UseTree::Name(name) => out.push(name.ident.to_string()),
        syn::UseTree::Rename(rename) => out.push(rename.rename.to_string()),
        syn::UseTree::Glob(_) => {}
        syn::UseTree::Group(group) => group.items.iter().for_each(|tree| collect_names(tree, out)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lower_crate_is_mirrored_through_its_library_name() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("crates/horizon-mod-render");
        std::fs::create_dir_all(package.join("src")).unwrap();
        std::fs::write(
            package.join("Cargo.toml"),
            "[package]\nname = 'horizon-mod-render'\n[lib]\nname = 'mod_render'\n",
        )
        .unwrap();
        std::fs::write(package.join("src/lib.rs"), "pub mod render {}\n").unwrap();
        let mut source = Source {
            root: root.path().to_owned(),
            loaded: BTreeSet::new(),
            files: BTreeMap::new(),
            parsed: BTreeMap::new(),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        assert!(source.load("horizon_mod_render"));
        assert_eq!(
            original_path(&source.libraries, "horizon_mod_render::render"),
            "::mod_render::render"
        );
        assert_eq!(
            original_path(&source.libraries, "horizon_client::game::autosave"),
            "::horizon_client::game::autosave"
        );
    }

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// [`generate`] on this checkout, with its function index for `targets`.
    fn generate_here(targets: &[String], extras: &Extras) -> Result<PatchCrate, Vec<Refusal>> {
        let generic = extras
            .loops
            .iter()
            .map(|l| crate::functions::key(&l.published.generic_target()))
            .collect();
        let only = targets.iter().map(|t| crate::functions::key(t)).collect();
        let index = crate::functions::index_of(&root(), &generic, Some(&only)).unwrap();
        generate(&root(), "horizon_patch", targets, extras, &index)
    }

    #[test]
    #[ignore = "daily"]
    fn a_lower_free_function_installs_through_its_library_name() {
        // `horizon-mod-render`'s library is `mod_render`: the installer must call the slot there.
        let crate_ = generate_here(
            &["horizon_mod_render::render::world_drawn".into()],
            &Extras::default(),
        )
        .unwrap();
        let lib: String = crate_.libs[0].1.split_whitespace().collect();
        assert!(
            lib.contains("::mod_render::render::__fnr_world_drawn(world_drawn)"),
            "{}",
            crate_.libs[0].1
        );
        assert!(
            !lib.contains("horizon_mod_render::render::__fnr_"),
            "{}",
            crate_.libs[0].1
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_method_moves_into_a_trait_of_its_own_type() {
        let crate_ = generate_here(
            &["horizon_client::game::autosave::Game<'_>::next_autosave_interval".into()],
            &Extras::default(),
        )
        .unwrap();
        assert_eq!(
            crate_.patched[0].id,
            "horizon_patch::game::autosave::<Game<'_> as __Fnr_Game_next_autosave_interval>::next_autosave_interval"
        );
        assert!(crate_.libs[0].1.contains("pub mod autosave"));
        assert!(
            crate_.libs[0]
                .1
                .contains("impl __Fnr_Game_next_autosave_interval for Game<'_>")
        );
        assert!(crate_.libs[0].1.contains("self.clock.sprinting()"));
    }

    #[test]
    #[ignore = "daily"]
    fn refusals_are_named() {
        let error = generate_here(
            &[
                "horizon_client::game::autosave::Game<'_>::no_such_function".into(),
                "horizon_no_such_crate::f".into(),
            ],
            &Extras::default(),
        )
        .unwrap_err();
        let codes: Vec<_> = error.iter().map(|r| r.code).collect();
        assert_eq!(codes, ["E_MIXIN_TARGET_MISSING", "E_MIXIN_TARGET_MISSING"]);
    }

    #[test]
    #[ignore = "daily"]
    fn a_lower_crate_function_gets_a_patch_crate_of_its_own() {
        let crate_ = generate_here(
            &[
                "horizon_client::game::autosave::Game<'_>::next_autosave_interval".into(),
                "horizon_client_lib::clock::FixedStep::advance".into(),
            ],
            &Extras::default(),
        )
        .unwrap();
        let names: Vec<_> = crate_.libs.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["horizon_patch", "horizon_patch_client_lib"]);
        let lower = &crate_.libs[1].1;
        assert!(lower.contains("pub mod clock"), "{lower}");
        assert!(
            lower.contains("pub use ::horizon_client_lib::clock::FixedStep;"),
            "{lower}"
        );
        assert!(crate_.patched.iter().any(|p| {
            p.id.starts_with("horizon_patch_client_lib::clock::<FixedStep as")
        }));
    }

    #[test]
    fn lower_ancestors_do_not_export_unwidened_private_items() {
        let mut source = Source {
            root: PathBuf::new(),
            loaded: BTreeSet::new(),
            files: BTreeMap::new(),
            parsed: BTreeMap::from([(
                "lower::ancestor".to_owned(),
                Module {
                    items: syn::parse_file("fn private_helper() {} pub fn public_helper() {}")
                        .unwrap()
                        .items,
                },
            )]),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        let ancestor = mirror(
            &mut source,
            "lower::ancestor",
            &Tree::default(),
            false,
            None,
        )
        .unwrap()
        .to_string();
        assert!(ancestor.contains("public_helper"));
        assert!(!ancestor.contains("private_helper"));
        let mut target = Tree::default();
        target.copies.push(quote!(
            pub fn copy() {}
        ));
        let target = mirror(&mut source, "lower::ancestor", &target, false, None)
            .unwrap()
            .to_string();
        assert!(target.contains("private_helper"));
        source.parsed.insert(
            "horizon_client::game".to_owned(),
            Module {
                items: syn::parse_file("fn game_helper() {}").unwrap().items,
            },
        );
        assert!(
            mirror(
                &mut source,
                "horizon_client::game",
                &Tree::default(),
                false,
                None
            )
            .unwrap()
            .to_string()
            .contains("game_helper")
        );
    }

    #[test]
    fn a_lower_crate_mirror_keeps_only_the_imports_its_copies_and_helpers_name() {
        let dir = std::env::temp_dir().join(format!("fnr-pruned-imports-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("lib.rs");
        std::fs::write(
            &file,
            "use std::sync::Arc; use std::rc::Rc; use child::Private; use std::collections::*;",
        )
        .unwrap();
        let mut source = Source {
            root: dir.clone(),
            loaded: BTreeSet::new(),
            files: BTreeMap::from([("fixture".to_owned(), vec![file])]),
            parsed: BTreeMap::new(),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        let used = BTreeSet::from(["Arc".to_owned()]);
        let out = mirror(&mut source, "fixture", &Tree::default(), false, Some(&used)).unwrap();
        let parsed: syn::File = syn::parse2(out).unwrap();
        let imports: Vec<String> = uses(&parsed.items)
            .iter()
            .map(|item| item.tree.to_token_stream().to_string())
            .collect();
        assert_eq!(imports, ["std :: sync :: Arc", "std :: collections :: *"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn included_files_keep_both_sets_of_module_imports() {
        let dir = std::env::temp_dir().join(format!("fnr-included-imports-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let outer = dir.join("mod.rs");
        let included = dir.join("shared.rs");
        std::fs::write(&outer, "use std::sync::Arc;").unwrap();
        std::fs::write(&included, "use std::rc::Rc;").unwrap();
        let mut source = Source {
            root: dir.clone(),
            loaded: BTreeSet::new(),
            files: BTreeMap::from([("fixture".to_owned(), vec![outer, included])]),
            parsed: BTreeMap::new(),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        let out = mirror(&mut source, "fixture", &Tree::default(), false, None).unwrap();
        let parsed: syn::File = syn::parse2(out).unwrap();
        let imports: Vec<String> = uses(&parsed.items)
            .iter()
            .map(|item| item.tree.to_token_stream().to_string())
            .collect();
        assert_eq!(imports, ["std :: sync :: Arc", "std :: rc :: Rc"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn support_modules_and_bridges_land_in_the_mirror_their_rows_name() {
        let extras = Extras {
            modules: vec![SupportModule {
                id: "support".into(),
                file: "crates/horizon-client/src/game/world/autosave.rs".into(),
                name: "fnr_support".into(),
                text: "pub fn twice(n: i64) -> i64 { n * 2 }".into(),
                public: false,
            }],
            bridges: vec![Bridge {
                id: "ticks".into(),
                method: "__horizon_mixins_m_ticks".into(),
                kind: "GET_MUT".into(),
                target: "horizon_client::game::Game".into(),
                member: Some("ticks_until_autosave".into()),
            }],
            idents: BTreeSet::new(),
            loops: Vec::new(),
        };
        let crate_ = generate_here(
            &["horizon_client::game::autosave::Game<'_>::next_autosave_interval".into()],
            &extras,
        )
        .unwrap();
        let lib = crate_.libs[0].1.replace(char::is_whitespace, "");
        assert!(
            lib.contains("pub(crate)modfnr_support{pubfntwice"),
            "{}",
            crate_.libs[0].1
        );
        assert!(
            lib.contains("fn__horizon_mixins_m_ticks(&mutself)->&mut"),
            "{}",
            crate_.libs[0].1
        );
        assert!(lib.contains("{&mutself.ticks_until_autosave}"));
        assert!(lib.contains(
            "pubmod__fnr_bridges{pubusecrate::game::__Fnr_Bridge___horizon_mixins_m_ticks;}"
        ));
        assert!(lib.contains("usecrate::__fnr_bridges::*;"));
        let error = generate_here(
            &[],
            &Extras {
                modules: vec![SupportModule {
                    file: "crates/horizon-client-lib/src/clock.rs".into(),
                    ..extras.modules[0].clone()
                }],
                bridges: vec![Bridge {
                    member: Some("no_such_field".into()),
                    ..extras.bridges[0].clone()
                }],
                idents: BTreeSet::new(),
                loops: Vec::new(),
            },
        )
        .unwrap_err();
        let codes: Vec<_> = error.iter().map(|r| r.code).collect();
        assert_eq!(
            codes,
            ["E_MIXIN_MODULE_DESTINATION", "E_MIXIN_BRIDGE_TARGET"]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_filled_loops_entry_runs_the_loop_with_the_mods_slots_in_order() {
        let published = crate::loops::collect(&root())
            .unwrap()
            .into_iter()
            .find(|l| l.name == "block_entities.tick")
            .expect("the game publishes block_entities.tick");
        let slot = |mod_id: &str, with: &str| LoopSlot {
            mod_id: mod_id.into(),
            id: "row".into(),
            helper: format!(
                "#[derive(Default)] pub struct {with}; impl BlockEntityTickSlot for {with} {{}}"
            ),
            with: with.into(),
        };
        let extras = Extras {
            loops: vec![LoopSlots {
                published: published.clone(),
                slots: vec![slot("first", "Count"), slot("second", "Skip")],
            }],
            ..Extras::default()
        };
        let crate_ = generate_here(std::slice::from_ref(&published.entry), &extras).unwrap();
        assert_eq!(crate_.patched.len(), 1);
        let lib = crate_.libs[0].1.replace(char::is_whitespace, "");
        // First by priority outside, then the next, then the empty slot.
        assert!(
            lib.contains("pubtype__FnrLoopSlots_block_entities_tick=(::horizon_client::game::published_loops::Attributed<__FnrLoopMod_0_block_entities_tick,__fnr_loop_0_block_entities_tick::Count,>,(::horizon_client::game::published_loops::Attributed<__FnrLoopMod_1_block_entities_tick,__fnr_loop_1_block_entities_tick::Skip,>,(),),);"),
            "{}",
            crate_.libs[0].1
        );
        assert!(lib.contains("pubmod__fnr_loop_1_block_entities_tick{usesuper::*;#[derive(Default)]pubstructSkip;implBlockEntityTickSlotforSkip{}}"), "{}", crate_.libs[0].1);
        assert!(
            lib.contains(
                "id:\"second\",hook:\"loopblock_entities.tick\",part:Some(\"second::Skip\")"
            ),
            "{}",
            crate_.libs[0].1
        );
        assert!(
            lib.contains("self.run_published_loop(\"block_entities.tick\","),
            "{}",
            crate_.libs[0].1
        );
        assert!(
            lib.contains("|game|game.tick_block_entities_with(&mut())"),
            "{}",
            crate_.libs[0].1
        );
        // A row on the loop's function: it is copied once, with no cell of its own, and the entry's
        // copy calls the copy.
        let generic = published.generic_target();
        let both = generate_here(&[published.entry.clone(), generic.clone()], &extras).unwrap();
        let installs: Vec<_> = both
            .patched
            .iter()
            .map(|p| (p.target.as_str(), p.install.is_some()))
            .collect();
        assert_eq!(
            installs,
            [(published.entry.as_str(), true), (generic.as_str(), false)]
        );
        let lib = both.libs[0].1.replace(char::is_whitespace, "");
        assert!(lib.contains("pubtrait__Fnr_Game_tick_block_entities_with{fntick_block_entities_with(&mutself,__a1:&mut__FnrLoopSlots_block_entities_tick,)->Result<()>;"), "{}", both.libs[0].1);
        assert!(lib.contains("|game,slots:&mut__FnrLoopSlots_block_entities_tick|{__Fnr_Game_tick_block_entities_with::tick_block_entities_with(game,slots,)}") || lib.contains("|game,slots:&mut__FnrLoopSlots_block_entities_tick|__Fnr_Game_tick_block_entities_with::tick_block_entities_with(game,slots,),"), "{}", both.libs[0].1);
        assert!(
            lib.contains("|game|game.tick_block_entities_with(&mut()),"),
            "{}",
            both.libs[0].1
        );
        assert!(
            !lib.contains("__fnr_install_Game_tick_block_entities_with"),
            "{}",
            both.libs[0].1
        );
        // The copy is for the one slot type: a path through the slot parameter names that type,
        // since the copy declares no `S` (`S::RECHECK` failed to build).
        assert!(
            lib.contains("if<__FnrLoopSlots_block_entities_tick>::RECHECK"),
            "{}",
            both.libs[0].1
        );
        assert!(!lib.contains("ifS::RECHECK"), "{}", both.libs[0].1);
        // The copy holds the loop itself, not a call to it: a call-site row inside the loop (the
        // check mod's selector) finds its one call there.
        assert_eq!(
            lib.matches("ticking(pos.0,pos.2)").count(),
            1,
            "{}",
            both.libs[0].1
        );
        // A helper that is not Rust is refused by name.
        let mut broken = extras.clone();
        broken.loops[0].slots[1].helper = "fn (".into();
        let refused = generate_here(std::slice::from_ref(&published.entry), &broken).unwrap_err();
        assert_eq!(refused[0].code, "E_LOOP_HELPER");
        assert!(
            refused[0].why.starts_with("second:row: its helper:"),
            "{}",
            refused[0].why
        );
    }
}

#[cfg(test)]
mod spike {
    /// Writes the patch crate for `FNR_TARGETS` (comma-separated) into `FNR_OUT`, for trying
    /// horizon-mixins on it by hand.
    #[test]
    #[ignore = "manual: writes a patch crate"]
    fn write_patch_crate() {
        let out = std::path::PathBuf::from(std::env::var("FNR_OUT").unwrap());
        let targets: Vec<String> = std::env::var("FNR_TARGETS")
            .unwrap()
            .split(',')
            .map(str::to_owned)
            .collect();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let only = targets.iter().map(|t| crate::functions::key(t)).collect();
        let index = crate::functions::index_of(&root, &Default::default(), Some(&only)).unwrap();
        match super::generate(
            &root,
            "horizon_patch",
            &targets,
            &super::Extras::default(),
            &index,
        ) {
            Ok(crate_) => {
                for (name, lib) in &crate_.libs {
                    std::fs::create_dir_all(out.join(name).join("src")).unwrap();
                    std::fs::write(out.join(name).join("src/lib.rs"), lib).unwrap();
                }
                for patched in crate_.patched {
                    println!(
                        "{} => {} | {:?}",
                        patched.target, patched.id, patched.install
                    );
                }
            }
            Err(refusals) => refusals.iter().for_each(|r| println!("REFUSED {r}")),
        }
    }
}

#[cfg(test)]
mod census {
    use super::*;

    /// Every function of the game module, by whether the copy route takes it and, if not, the
    /// refusal it gets. `cargo test -p horizon-mod-api-gen --lib census -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement: prints the census"]
    fn refusal_census() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut source = Source::new(&root).unwrap();
        let modules: Vec<String> = source.files.keys().cloned().collect();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut total = 0;
        // Call sites in copyable bodies: all, beneath a borrow, borrowing a call themselves.
        let mut sites = [0usize; 3];
        let mut copyable: Vec<String> = Vec::new();
        let mut note = |why: String| *counts.entry(why).or_default() += 1;
        for module in modules
            .iter()
            .filter(|m| m.starts_with("horizon_client::game"))
        {
            let items = source.module(module).unwrap().items.clone();
            let mut functions: Vec<Found> = Vec::new();
            fn walk(items: &[Item], module: &str, out: &mut Vec<Found>, traits: &mut usize) {
                for item in items {
                    match item {
                        Item::Fn(f) => out.push(Found {
                            module: module.into(),
                            owner: None,
                            function: syn::ImplItemFn {
                                attrs: f.attrs.clone(),
                                vis: f.vis.clone(),
                                defaultness: None,
                                sig: f.sig.clone(),
                                block: (*f.block).clone(),
                            },
                        }),
                        Item::Impl(block) if block.trait_.is_some() => {
                            *traits += block
                                .items
                                .iter()
                                .filter(|i| matches!(i, ImplItem::Fn(_)))
                                .count();
                        }
                        Item::Impl(block) => {
                            for member in &block.items {
                                if let ImplItem::Fn(f) = member {
                                    out.push(Found {
                                        module: module.into(),
                                        owner: Some(block.clone()),
                                        function: f.clone(),
                                    });
                                }
                            }
                        }
                        Item::Mod(inline) if inline.content.is_some() => {
                            let test = inline
                                .attrs
                                .iter()
                                .any(|a| a.to_token_stream().to_string().contains("test"));
                            if !test {
                                walk(
                                    &inline.content.as_ref().unwrap().1,
                                    &format!("{module}::{}", inline.ident),
                                    out,
                                    traits,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            let mut traits = 0;
            walk(&items, module, &mut functions, &mut traits);
            total += traits;
            for _ in 0..traits {
                note("refused: trait method (not enumerated)".into());
            }
            for found in functions {
                let attrs = &found.function.attrs;
                if attrs.iter().any(|a| {
                    a.path().is_ident("test")
                        || (a.path().is_ident("cfg")
                            && a.to_token_stream().to_string().contains("test"))
                }) {
                    continue;
                }
                total += 1;
                let sig = &found.function.sig;
                let why = if attrs
                    .iter()
                    .any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
                    || found
                        .owner
                        .as_ref()
                        .is_some_and(|b| b.attrs.iter().any(|a| a.path().is_ident("cfg")))
                {
                    "refused: compiled only under a cfg".to_owned()
                } else if !crate::replace::eligible(attrs, sig) {
                    "refused: const/async/unsafe/self by value/impl Trait or ! /inline(always)"
                        .to_owned()
                } else if !(crate::replace::lifetimes_only(&sig.generics)
                    && found
                        .owner
                        .as_ref()
                        .is_none_or(|b| crate::replace::lifetimes_only(&b.generics)))
                {
                    "refused: generic over types".to_owned()
                } else {
                    if replaceable(&found, &mut source, false).is_ok() {
                        let owner = found.owner.as_ref().map(|b| {
                            format!("{}::", plain(&b.self_ty.to_token_stream().to_string()))
                        });
                        copyable.push(format!(
                            "{}::{}{}",
                            found.module,
                            owner.unwrap_or_default(),
                            found.function.sig.ident
                        ));
                        let mut s = CallSites::default();
                        s.visit_block(&found.function.block);
                        sites[0] += s.all;
                        sites[1] += s.beneath;
                        sites[2] += s.borrowing;
                    }
                    match replaceable(&found, &mut source, false) {
                        Ok(()) if referenced_call(&found.function.block) => {
                            "copied, but upstream refuses call-site kinds: `&` over a call"
                                .to_owned()
                        }
                        Ok(()) => "copied".to_owned(),
                        Err(why) => format!("refused: {}", why.split(':').next().unwrap_or(&why)),
                    }
                };
                note(why);
            }
        }
        println!("game-module functions: {total}");
        if let Ok(list) = std::env::var("FNR_CENSUS_LIST") {
            std::fs::write(list, copyable.join("\n")).unwrap();
        }
        println!(
            "call sites in copyable functions: {}; beneath `&` (value-wrapping kinds only with the \
             carried narrowing): {}; borrowing a call in their own expression (refused): {}",
            sites[0], sites[1], sites[2]
        );
        #[derive(Default)]
        struct CallSites {
            all: usize,
            beneath: usize,
            borrowing: usize,
            references: usize,
        }
        impl<'ast> Visit<'ast> for CallSites {
            fn visit_expr(&mut self, e: &'ast syn::Expr) {
                if matches!(e, syn::Expr::Call(_) | syn::Expr::MethodCall(_)) {
                    self.all += 1;
                    self.beneath += usize::from(self.references > 0);
                    self.borrowing += usize::from(referenced_call_in(e));
                }
                let reference = matches!(e, syn::Expr::Reference(_));
                self.references += usize::from(reference);
                syn::visit::visit_expr(self, e);
                self.references -= usize::from(reference);
            }
            fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
            fn visit_macro(&mut self, _: &'ast syn::Macro) {}
        }
        fn referenced_call_in(e: &syn::Expr) -> bool {
            let block = syn::Block {
                brace_token: Default::default(),
                stmts: vec![syn::Stmt::Expr(e.clone(), None)],
            };
            referenced_call(&block)
        }
        fn referenced_call(block: &syn::Block) -> bool {
            // horizon-mixins' rule: a call beneath `&` outside macro arguments refuses every
            // call-site kind on the function.
            struct Find(bool);
            impl<'ast> Visit<'ast> for Find {
                fn visit_expr(&mut self, e: &'ast syn::Expr) {
                    if let syn::Expr::Reference(r) = e {
                        struct Call(bool);
                        impl<'ast> Visit<'ast> for Call {
                            fn visit_expr(&mut self, e: &'ast syn::Expr) {
                                if matches!(e, syn::Expr::Call(_) | syn::Expr::MethodCall(_)) {
                                    self.0 = true;
                                }
                                syn::visit::visit_expr(self, e);
                            }
                            fn visit_macro(&mut self, _: &'ast syn::Macro) {}
                        }
                        let mut c = Call(false);
                        c.visit_expr(&r.expr);
                        self.0 |= c.0;
                    }
                    syn::visit::visit_expr(self, e);
                }
                fn visit_macro(&mut self, _: &'ast syn::Macro) {}
            }
            let mut f = Find(false);
            f.visit_block(block);
            f.0
        }
        for (why, n) in counts {
            println!("  {why}: {n} ({:.1}%)", 100.0 * n as f64 / total as f64);
        }
    }

    #[test]
    fn an_invokers_self_items_are_qualified_or_refused_by_name() {
        let owner: syn::ItemImpl = syn::parse_quote! {
            impl Convert for Bar {
                type Error = Failure;
                fn convert(&self) -> Result<u8, Self::Error> { Ok(0) }
            }
        };
        let mut sig: syn::Signature =
            syn::parse_quote!(fn convert(&self) -> Result<u8, Self::Error>);
        invoker_self_items(&mut sig, &owner).unwrap();
        assert_eq!(
            sig.to_token_stream().to_string().replace(' ', ""),
            "fnconvert(&self)->Result<u8,<BarasConvert>::Error>"
        );
        // An item the `impl` does not define has no spelling the bridge's trait could use.
        let mut sig: syn::Signature = syn::parse_quote!(fn convert(&self) -> Self::Other);
        assert_eq!(
            invoker_self_items(&mut sig, &owner).unwrap_err(),
            "an invoker's signature names `Self::Other`, which a bridge cannot name; bridge a function whose signature spells the type out"
        );
    }
}
