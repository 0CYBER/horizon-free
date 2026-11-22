//! The Rust items the mod API's signatures name, found as rustc finds them: through each
//! module's items, `use`s and glob imports, into the crates `horizon-mod-api` builds on.
use std::{collections::HashMap, fs, path::Path};
use syn::{Item, UseTree};

/// The crates whose items mod API signatures name: crate name and source directory.
const CRATES: &[(&str, &str)] = &[
    ("mod_api", "crates/horizon-mod-vocab/src"),
    ("horizon_core", "crates/horizon-core/src"),
    ("horizon_mod_runtime", "crates/horizon-mod-runtime/src"),
    ("horizon_world", "crates/horizon-world/src"),
    ("horizon_items", "crates/horizon-items/src"),
    ("horizon_client_core", "crates/horizon-client-core/src"),
    ("horizon_assets", "crates/horizon-assets/src"),
    ("horizon_mod_package", "crates/horizon-mod-package/src"),
];

/// One module: its crate, its path below the crate root, and its items.
pub(super) struct Module {
    pub(super) krate: &'static str,
    pub(super) path: Vec<String>,
    pub(super) items: Vec<Item>,
}

/// A named item a signature refers to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct ItemId {
    pub(super) module: usize,
    pub(super) item: usize,
}

/// Where a name leads.
pub(super) enum Found {
    Item(ItemId),
    /// A path outside the indexed crates (`std::…`, `serde_json::…`), as written.
    External(Vec<String>),
}

/// The indexed crates' modules, parsed as they are first reached.
pub(super) struct Index<'a> {
    root: &'a Path,
    /// Sources included into an inline module with `include!`, by file name.
    included: &'a [(&'a str, &'a str)],
    pub(super) modules: Vec<Module>,
    by_path: HashMap<(&'static str, Vec<String>), usize>,
    /// How deep `resolve` is in following imports.
    depth: usize,
    /// Every source file read, for the build script to track.
    pub(super) read: std::cell::RefCell<Vec<std::path::PathBuf>>,
    /// Crates added for one consumer beside [`CRATES`] ([`Index::add_crate`]).
    extra: Vec<(&'static str, &'static str)>,
}

/// The file a module's `#[path = "..."]` names.
fn path_attribute(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| match &attr.meta {
        syn::Meta::NameValue(meta) if meta.path.is_ident("path") => match &meta.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(file),
                ..
            }) => Some(file.value()),
            _ => None,
        },
        _ => None,
    })
}

/// The directory below the crate root that holds `path`'s last module, with its slash.
fn parent_dir(path: &[String]) -> String {
    path.split_last().map_or(String::new(), |(_, dir)| {
        dir.iter().map(|d| format!("{d}/")).collect()
    })
}

impl<'a> Index<'a> {
    pub(super) fn new(root: &'a Path, included: &'a [(&'a str, &'a str)]) -> Self {
        Self {
            root,
            included,
            modules: Vec::new(),
            by_path: HashMap::new(),
            depth: 0,
            read: Default::default(),
            extra: Vec::new(),
        }
    }

    /// Indexes one more crate, for this index only: the surface walk reads the facade crate
    /// (`horizon_mod_api`), which no generated output resolves through.
    pub(super) fn add_crate(&mut self, name: &'static str, dir: &'static str) {
        if self.crate_dir(name).is_none() {
            self.extra.push((name, dir));
        }
    }

    fn crate_dir(&self, name: &str) -> Option<(&'static str, &'static str)> {
        CRATES
            .iter()
            .chain(&self.extra)
            .find(|(krate, _)| *krate == name)
            .copied()
    }

    pub(super) fn item(&self, id: ItemId) -> &Item {
        &self.modules[id.module].items[id.item]
    }

    /// The module at `path` in `krate`, parsed on first use.
    pub(super) fn module(&mut self, krate: &str, path: &[String]) -> Result<usize, String> {
        let krate = self
            .crate_dir(krate)
            .map(|(name, _)| name)
            .ok_or_else(|| format!("{krate} is not a crate the mod API builds on"))?;
        if let Some(&id) = self.by_path.get(&(krate, path.to_vec())) {
            return Ok(id);
        }
        let items = match path.split_last() {
            None => self.parse(krate, "lib.rs")?,
            Some((last, parent)) => {
                let parent = self.module(krate, parent)?;
                let declared = self.modules[parent]
                    .items
                    .iter()
                    .find_map(|item| match item {
                        Item::Mod(item) if item.ident == last.as_str() => {
                            Some((item.content.clone(), path_attribute(&item.attrs)))
                        }
                        _ => None,
                    });
                match declared {
                    Some((Some((_, items)), _)) => self.expand_includes(items)?,
                    // A generated `#[path]` is absolute.
                    Some((None, Some(file))) if Path::new(&file).is_absolute() => {
                        self.parse(krate, &file)?
                    }
                    // `#[path]` is relative to the declaring file's directory: `parent.rs`
                    // sits beside it, `parent/mod.rs` below.
                    Some((None, Some(file))) => {
                        let beside = parent_dir(&path[..path.len() - 1]);
                        let below = parent_dir(path);
                        self.parse(krate, &format!("{beside}{file}"))
                            .or_else(|_| self.parse(krate, &format!("{below}{file}")))?
                    }
                    _ => {
                        let dir = path.join("/");
                        self.parse(krate, &format!("{dir}.rs"))
                            .or_else(|_| self.parse(krate, &format!("{dir}/mod.rs")))?
                    }
                }
            }
        };
        let id = self.modules.len();
        self.modules.push(Module {
            krate,
            path: path.to_vec(),
            items,
        });
        self.by_path.insert((krate, path.to_vec()), id);
        Ok(id)
    }

    fn parse(&self, krate: &str, file: &str) -> Result<Vec<Item>, String> {
        let dir = self.crate_dir(krate).expect("a known crate").1;
        let path = self.root.join(dir).join(file);
        let source = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.read.borrow_mut().push(path.clone());
        let source = crate::exports::expand(self.root, &source)?;
        let file = syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
        self.expand_includes(file.items)
    }

    /// `items` with each `include!` of a generated file replaced by that file's items.
    fn expand_includes(&self, items: Vec<Item>) -> Result<Vec<Item>, String> {
        let mut out = Vec::new();
        for item in items {
            match &item {
                Item::Macro(mac) if mac.mac.path.is_ident("include") => {
                    let tokens = mac.mac.tokens.to_string();
                    // The wire impls are generated from what this reads, so it never reads them.
                    if tokens.contains("wire_") {
                        continue;
                    }
                    let (name, source) = self
                        .included
                        .iter()
                        .find(|(name, _)| tokens.contains(name))
                        .ok_or_else(|| format!("include!({tokens}) is not a generated file"))?;
                    let file = syn::parse_file(source).map_err(|e| format!("{name}: {e}"))?;
                    out.extend(file.items);
                }
                _ => out.push(item),
            }
        }
        Ok(out)
    }

    /// What `name` means in `module`: an item defined there, a `use` of it, or a glob import
    /// that provides it.
    pub(super) fn resolve(&mut self, module: usize, name: &str) -> Result<Found, String> {
        self.depth += 1;
        if self.depth > 64 {
            self.depth = 0;
            let m = &self.modules[module];
            return Err(format!(
                "{name} in {}::{} imports itself",
                m.krate,
                m.path.join("::")
            ));
        }
        let found = self.lookup(module, name, &mut Vec::new());
        self.depth = self.depth.saturating_sub(1);
        found?.ok_or_else(|| {
            let m = &self.modules[module];
            format!("{name} not found in {}::{}", m.krate, m.path.join("::"))
        })
    }

    fn lookup(
        &mut self,
        module: usize,
        name: &str,
        seen: &mut Vec<usize>,
    ) -> Result<Option<Found>, String> {
        if seen.contains(&module) {
            return Ok(None);
        }
        seen.push(module);
        let items = &self.modules[module].items;
        if let Some(item) = items.iter().position(|item| defines(item, name)) {
            return Ok(Some(Found::Item(ItemId { module, item })));
        }
        let mut named = None;
        let mut globs = Vec::new();
        for item in items {
            if let Item::Use(item) = item {
                uses(
                    &item.tree,
                    &mut Vec::new(),
                    &mut |path, alias| match alias {
                        Some(alias) if alias == name && named.is_none() => named = Some(path),
                        None => globs.push(path),
                        _ => {}
                    },
                );
            }
        }
        if let Some(path) = named {
            return self.resolve_path(module, &path).map(Some);
        }
        for glob in globs {
            if let Some((krate, path)) = self.absolute(module, &glob)
                && let Ok(target) = self.module(krate, &path)
                && let Some(found) = self.lookup(target, name, seen)?
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    /// What the path `path`, written in `module`, names.
    pub(super) fn resolve_path(&mut self, module: usize, path: &[String]) -> Result<Found, String> {
        if path.len() == 1 {
            return self.resolve(module, &path[0]);
        }
        let (last, parent) = path.split_last().unwrap();
        match self.absolute(module, parent) {
            Some((krate, parent)) => {
                // A path through a type (`Enum::Variant`) is not a module.
                let target = self.module(krate, &parent)?;
                self.resolve(target, last)
            }
            None => {
                // The first segment may be a name `use`d into `module` (a module alias).
                if let Ok(Found::External(prefix)) = self.resolve(module, &path[0])
                    && prefix.len() > 1
                {
                    let mut full = prefix;
                    full.extend(path[1..].iter().cloned());
                    return Ok(Found::External(full));
                }
                Ok(Found::External(path.to_vec()))
            }
        }
    }

    /// `path`, written in `module`, as a module of an indexed crate, if it is one.
    pub(super) fn absolute(
        &self,
        module: usize,
        path: &[String],
    ) -> Option<(&'static str, Vec<String>)> {
        let m = &self.modules[module];
        let (first, rest) = path.split_first()?;
        let mut base = match first.as_str() {
            "crate" => (m.krate, Vec::new()),
            "self" => (m.krate, m.path.clone()),
            "super" => (m.krate, m.path[..m.path.len().checked_sub(1)?].to_vec()),
            name if m
                .items
                .iter()
                .any(|item| matches!(item, Item::Mod(item) if item.ident == name)) =>
            {
                let mut path = m.path.clone();
                path.push(name.to_string());
                (m.krate, path)
            }
            name => (self.crate_dir(name)?.0, Vec::new()),
        };
        for segment in rest {
            if segment == "super" {
                base.1.pop()?;
            } else {
                base.1.push(segment.clone());
            }
        }
        Some(base)
    }
}

/// Whether `item` defines the type or trait `name`.
fn defines(item: &Item, name: &str) -> bool {
    match item {
        Item::Struct(item) => item.ident == name,
        Item::Enum(item) => item.ident == name,
        Item::Type(item) => item.ident == name,
        Item::Trait(item) => item.ident == name,
        Item::Union(item) => item.ident == name,
        Item::Const(item) => item.ident == name,
        _ => false,
    }
}

/// Each path `tree` imports, with the name it binds (`None` for a glob).
pub(super) fn uses(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    f: &mut impl FnMut(Vec<String>, Option<String>),
) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            uses(&path.tree, prefix, f);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            if name.ident != "self" {
                path.push(name.ident.to_string());
            }
            let alias = path.last().cloned();
            f(path, alias);
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            if rename.ident != "self" {
                path.push(rename.ident.to_string());
            }
            f(path, Some(rename.rename.to_string()));
        }
        UseTree::Glob(_) => f(prefix.clone(), None),
        UseTree::Group(group) => {
            for tree in &group.items {
                uses(tree, prefix, f);
            }
        }
    }
}
