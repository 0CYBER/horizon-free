//! The game modules a patch crate mirrors beyond its copies' own: the prebuilt game widens the
//! items its modules define, never their `use`s, so a path that reaches a name through a module's
//! import (`super::minecarts::CartKind`, which `minecarts` takes from `horizon_gameplay` with a
//! `pub(in crate::game) use`) is a private import in the prebuilt game (E0603). Mirroring that
//! module repeats its `use`s in the patch crate, where they resolve as they did in the game. The
//! paths read are the mirrored modules' `use`s and the copies' tokens, macro arguments included;
//! each newly mirrored module's `use`s are read in turn.
use super::{Source, Tree};
use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::Item;

/// How many imports a path is followed through before it is given up on.
const DEPTH: usize = 16;

/// Adds to `tree`, the mirror of crate `krate`, every module of it that the mirror's `use`s or
/// copies reach through an import, until none is added.
pub(super) fn extend(source: &mut Source, krate: &str, tree: &mut Tree) {
    let mut read = BTreeSet::new();
    loop {
        let mut mirrored = BTreeSet::new();
        nodes(tree, krate, &mut mirrored);
        let pending: Vec<String> = mirrored.difference(&read).cloned().collect();
        if pending.is_empty() {
            return;
        }
        let mut reach = Reach {
            source,
            mirrored,
            needed: BTreeSet::new(),
        };
        for module in pending {
            let Ok(items) = reach.source.module(&module).map(|m| m.items.clone()) else {
                read.insert(module);
                continue;
            };
            let mut paths = Vec::new();
            for item in &items {
                // A `use` under a `cfg` is left out: it may name a module the build does not have,
                // whose mirror would not compile. None of the game's modules has one that does.
                if let Item::Use(item) = item
                    && !item.attrs.iter().any(|attr| attr.path().is_ident("cfg"))
                    && item.leading_colon.is_none()
                {
                    let mut found = Vec::new();
                    leaves(&item.tree, &mut Vec::new(), &mut found);
                    paths.extend(
                        found
                            .into_iter()
                            .map(|leaf| (leaf.path, leaf.name.is_none())),
                    );
                }
            }
            let mut tokens = TokenStream::new();
            if let Some(node) = node(tree, krate, &module) {
                tokens.extend(node.copies.iter().cloned());
            }
            for mac in reach.source.scope_macros(&module) {
                mac.to_tokens(&mut tokens);
            }
            scan(tokens, 0, &Scope::new(), &mut paths);
            for (path, glob) in paths {
                reach.resolve(&module, &path, glob, 0);
            }
            read.insert(module);
        }
        let Reach { needed, .. } = reach;
        for module in needed {
            let mut node = &mut *tree;
            for segment in module.split("::").skip(1) {
                node = node.children.entry(segment.to_owned()).or_default();
            }
        }
    }
}

/// Whether `path`, written in module `at`, names a module of a loaded crate through modules
/// alone (`subtitles` in `use subtitles::{Subtitles, subtitles};`, not `subtitles::subtitles`).
pub(super) fn names_module(source: &mut Source, at: &str, path: &[String]) -> bool {
    let mut reach = Reach {
        source,
        mirrored: BTreeSet::new(),
        needed: BTreeSet::new(),
    };
    reach.module_of(at, path).is_some() || reach.source.is_module(&path.join("::"))
}

struct Reach<'a> {
    source: &'a mut Source,
    mirrored: BTreeSet<String>,
    /// The modules found to need a mirror this round.
    needed: BTreeSet<String>,
}

impl Reach<'_> {
    fn mirrored(&self, module: &str) -> bool {
        self.mirrored.contains(module) || self.needed.contains(module)
    }

    fn child(&mut self, module: &str, name: &str) -> Option<String> {
        let child = format!("{module}::{name}");
        self.source.is_module(&child).then_some(child)
    }

    /// The module of the crate `path`, written in module `at`, names through modules alone.
    fn module_of(&mut self, at: &str, path: &[String]) -> Option<String> {
        let mut module = at.to_owned();
        for (index, segment) in path.iter().enumerate() {
            module = match segment.as_str() {
                "crate" if index == 0 => at.split("::").next().unwrap_or(at).to_owned(),
                "self" if index == 0 => module,
                "super" => module.rsplit_once("::")?.0.to_owned(),
                name => self.child(&module, name)?,
            };
        }
        Some(module)
    }

    /// Follows `path`, written in module `at`, through the crate's modules; `glob` for a
    /// `use path::*`. A module whose import the path passes through is marked to be mirrored.
    /// Returns whether the path ends on an import of a mirror: a public import that forwards to
    /// one (`pub use super::minecarts as carts;`) needs its own module mirrored too, or a path
    /// through it would reach the prebuilt game's module instead of the mirror.
    fn resolve(&mut self, at: &str, path: &[String], glob: bool, depth: usize) -> bool {
        if depth > DEPTH || path.is_empty() {
            return false;
        }
        let first = path[0].as_str();
        let (mut module, rest) = match first {
            "crate" => (at.split("::").next().unwrap_or(at).to_owned(), &path[1..]),
            "self" => (at.to_owned(), &path[1..]),
            "super" => {
                let ups = path.iter().take_while(|s| *s == "super").count();
                let mut module = at;
                for _ in 0..ups {
                    match module.rsplit_once("::") {
                        Some((parent, _)) => module = parent,
                        None => return false,
                    }
                }
                (module.to_owned(), &path[ups..])
            }
            name => {
                if let Some(child) = self.child(at, name) {
                    (child, &path[1..])
                } else {
                    // A name `at` imports: followed from where the import takes it.
                    let Ok(items) = self.source.module(at).map(|m| m.items.clone()) else {
                        return false;
                    };
                    let from = import_of(&items, name)
                        .map(|(import, _)| import)
                        .or_else(|| {
                            // Or a glob of a module of the crate that has it (`use super::*;`).
                            globs(&items).into_iter().find_map(|mut prefix| {
                                let module = self.module_of(at, &prefix)?;
                                let has = self.child(&module, name).is_some()
                                    || self.source.module(&module).is_ok_and(|m| {
                                        defines(&m.items, name)
                                            || import_of(&m.items, name).is_some()
                                    });
                                has.then(|| {
                                    prefix.push(name.to_owned());
                                    prefix
                                })
                            })
                        });
                    let Some(from) = from else {
                        return false;
                    };
                    let joined: Vec<String> =
                        from.into_iter().chain(path[1..].iter().cloned()).collect();
                    return self.resolve(at, &joined, glob, depth + 1);
                }
            }
        };
        for (index, segment) in rest.iter().enumerate() {
            if let Some(child) = self.child(&module, segment) {
                module = child;
                continue;
            }
            let Ok(items) = self.source.module(&module).map(|m| m.items.clone()) else {
                return false;
            };
            if defines(&items, segment) {
                return false;
            }
            let after = &rest[index + 1..];
            let mirrored = self.mirrored(&module);
            return match import_of(&items, segment) {
                // A mirror's own import resolves in the patch crate; the path goes on from its
                // source.
                Some((import, _)) if mirrored => {
                    let joined: Vec<String> =
                        import.into_iter().chain(after.iter().cloned()).collect();
                    self.resolve(&module, &joined, glob, depth + 1);
                    true
                }
                // A public import is public in the prebuilt game too, unless what it forwards to
                // ends on a mirror's import.
                Some((import, true)) => {
                    let joined: Vec<String> =
                        import.into_iter().chain(after.iter().cloned()).collect();
                    let through = self.resolve(&module, &joined, glob, depth + 1);
                    if through {
                        self.needed.insert(module);
                    }
                    through
                }
                Some(_) => {
                    self.needed.insert(module);
                    true
                }
                // Through a glob import, or not the module's at all.
                None if mirrored => true,
                None if private_globs(&items) => {
                    self.needed.insert(module);
                    true
                }
                None => false,
            };
        }
        // The whole of `module`: a glob of it takes its imports too, which a mirror repeats.
        let imports = glob
            && self.source.module(&module).is_ok_and(|m| {
                m.items.iter().any(
                    |item| matches!(item, Item::Use(u) if !test_only(&u.attrs) && !public(&u.vis)),
                )
            });
        if imports && !self.mirrored(&module) {
            self.needed.insert(module);
        }
        imports
    }
}

/// The paths of the modules in `tree`, the mirror of `path`.
fn nodes(tree: &Tree, path: &str, out: &mut BTreeSet<String>) {
    out.insert(path.to_owned());
    for (name, child) in &tree.children {
        nodes(child, &format!("{path}::{name}"), out);
    }
}

fn node<'a>(tree: &'a Tree, krate: &str, module: &str) -> Option<&'a Tree> {
    let rest = module.strip_prefix(krate)?;
    let mut node = tree;
    for segment in rest.split("::").filter(|s| !s.is_empty()) {
        node = node.children.get(segment)?;
    }
    Some(node)
}

fn public(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || (attr.path().is_ident("cfg") && attr.to_token_stream().to_string().contains("test"))
    })
}

/// Whether `items` define `name` themselves: an item the prebuilt game makes public.
fn defines(items: &[Item], name: &str) -> bool {
    items.iter().any(|item| {
        let ident = match item {
            Item::Fn(item) => &item.sig.ident,
            Item::Struct(item) => &item.ident,
            Item::Enum(item) => &item.ident,
            Item::Union(item) => &item.ident,
            Item::Const(item) => &item.ident,
            Item::Static(item) => &item.ident,
            Item::Type(item) => &item.ident,
            Item::Trait(item) => &item.ident,
            Item::Mod(item) => &item.ident,
            _ => return false,
        };
        ident == name
    })
}

/// Where the explicit `use` of `items` that binds `name` takes it from, and whether it is public.
fn import_of(items: &[Item], name: &str) -> Option<(Vec<String>, bool)> {
    items.iter().find_map(|item| match item {
        Item::Use(item) if !test_only(&item.attrs) && item.leading_colon.is_none() => {
            let mut found = Vec::new();
            leaves(&item.tree, &mut Vec::new(), &mut found);
            found
                .into_iter()
                .find(|leaf| leaf.name.as_deref() == Some(name))
                .map(|leaf| (leaf.path, public(&item.vis)))
        }
        _ => None,
    })
}

/// The paths of the glob imports of `items` (`use super::*;` is `[super]`).
fn globs(items: &[Item]) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    for item in items {
        if let Item::Use(item) = item
            && !test_only(&item.attrs)
            && item.leading_colon.is_none()
        {
            leaves(&item.tree, &mut Vec::new(), &mut found);
        }
    }
    found
        .into_iter()
        .filter(|leaf| leaf.name.is_none())
        .map(|leaf| leaf.path)
        .collect()
}

/// Whether `items` have a glob import that is not public.
fn private_globs(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Use(item) if !test_only(&item.attrs) && !public(&item.vis) => {
            let mut found = Vec::new();
            leaves(&item.tree, &mut Vec::new(), &mut found);
            found.iter().any(|leaf| leaf.name.is_none())
        }
        _ => false,
    })
}

/// One leaf of a `use` tree: the path it imports and the name it binds, none for a glob
/// (`a::b::*` is `[a, b]`).
struct Leaf {
    path: Vec<String>,
    name: Option<String>,
}

fn leaves(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<Leaf>) {
    let leaf = |path: Vec<String>, name: Option<String>| Leaf { path, name };
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            leaves(&path.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) if name.ident == "self" => {
            out.push(leaf(prefix.clone(), prefix.last().cloned()));
        }
        syn::UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(name.ident.to_string());
            out.push(leaf(path, Some(name.ident.to_string())));
        }
        syn::UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            if rename.ident != "self" {
                path.push(rename.ident.to_string());
            }
            out.push(leaf(path, Some(rename.rename.to_string())));
        }
        syn::UseTree::Glob(_) => out.push(leaf(prefix.clone(), None)),
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                leaves(tree, prefix, out);
            }
        }
    }
}

/// The names a block's or module's own `use`s bind, as paths from the copy's module; `None` for
/// a name bound to something the scan does not follow, which hides an outer binding.
type Scope = BTreeMap<String, Option<Vec<String>>>;

/// Reads `tokens`, `depth` inline modules (`mod helper { … }`) below the copy's module, into
/// `out`: each path of two or more segments (`a::b::C`), macro arguments included, as it reads
/// from the copy's module, with whether it is a glob `use`. A name a `use` of the block or of an
/// enclosing block binds (`use super::minecarts as carts;`) is followed to what it binds; an
/// inline module sees only its own. A path that stays inside the inline modules is left out, and
/// so is an item under a `cfg` attribute: it may name a module the build does not have. An
/// absolute path (`::std::…`) and a macro variable's (`$x::…`) are not the crate's; `$crate::…` is
/// `crate::…`.
fn scan(tokens: TokenStream, depth: usize, outer: &Scope, out: &mut Vec<(Vec<String>, bool)>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    // The block's `use`s first: their names hold in all of it, before and after them.
    let mut scope = outer.clone();
    let mut index = 0;
    while index < trees.len() {
        if let Some(end) = gated(&trees, index) {
            index = end;
        } else if starts_use(&trees, index) {
            let end = statement_end(&trees, index);
            if let Some(item) = use_statement(&trees[index..end]) {
                let mut found = Vec::new();
                leaves(&item.tree, &mut Vec::new(), &mut found);
                for leaf in found {
                    let path = local(&scope, &leaf.path, depth);
                    if let Some(name) = &leaf.name {
                        scope.insert(name.clone(), path.clone());
                    }
                    if let Some(path) = path {
                        out.push((path, leaf.name.is_none()));
                    }
                }
            }
            index = end;
        } else {
            index += 1;
        }
    }
    let mut index = 0;
    while index < trees.len() {
        if let Some(end) = gated(&trees, index) {
            index = end;
            continue;
        }
        if starts_use(&trees, index) {
            index = statement_end(&trees, index);
            continue;
        }
        match &trees[index] {
            TokenTree::Group(group) => {
                scan(group.stream(), depth, &scope, out);
                index += 1;
            }
            TokenTree::Ident(ident)
                if ident == "mod"
                    && matches!(trees.get(index + 1), Some(TokenTree::Ident(_)))
                    && matches!(trees.get(index + 2), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace) =>
            {
                if let Some(TokenTree::Group(body)) = trees.get(index + 2) {
                    scan(body.stream(), depth + 1, &Scope::new(), out);
                }
                index += 3;
            }
            TokenTree::Ident(ident) => {
                let after_colons = index >= 2 && colons(&trees, index - 2);
                let after_dollar = index >= 1 && punct(&trees, index - 1, '$');
                let mut path = vec![ident.to_string()];
                let mut next = index + 1;
                while colons(&trees, next)
                    && let Some(TokenTree::Ident(segment)) = trees.get(next + 2)
                {
                    path.push(segment.to_string());
                    next += 3;
                }
                if path.len() > 1
                    && !after_colons
                    && (!after_dollar || path[0] == "crate")
                    && let Some(path) = local(&scope, &path, depth)
                {
                    out.push((path, false));
                }
                index = next;
            }
            _ => index += 1,
        }
    }
}

fn punct(trees: &[TokenTree], at: usize, c: char) -> bool {
    matches!(trees.get(at), Some(TokenTree::Punct(p)) if p.as_char() == c)
}

fn colons(trees: &[TokenTree], at: usize) -> bool {
    matches!(trees.get(at), Some(TokenTree::Punct(p)) if p.as_char() == ':' && p.spacing() == Spacing::Joint)
        && punct(trees, at + 1, ':')
}

fn starts_use(trees: &[TokenTree], at: usize) -> bool {
    matches!(&trees[at], TokenTree::Ident(ident) if ident == "use")
        && !(at >= 2 && colons(trees, at - 2))
}

/// Where the statement starting at `at` ends: past its `;`.
fn statement_end(trees: &[TokenTree], at: usize) -> usize {
    (at..trees.len())
        .find(|&end| punct(trees, end, ';'))
        .map_or(trees.len(), |end| end + 1)
}

/// The `use` statement `tokens`, `$crate` read as `crate`.
fn use_statement(tokens: &[TokenTree]) -> Option<syn::ItemUse> {
    let statement: TokenStream = tokens
        .iter()
        .enumerate()
        .filter(|(at, tree)| {
            !(matches!(tree, TokenTree::Punct(p) if p.as_char() == '$')
                && matches!(tokens.get(at + 1), Some(TokenTree::Ident(i)) if i == "crate"))
        })
        .map(|(_, tree)| tree.clone())
        .collect();
    syn::parse2::<syn::ItemUse>(statement)
        .ok()
        .filter(|item| item.leading_colon.is_none())
}

/// Where the item, statement, field or arm under a `cfg` attribute at `at` ends (past its `;`,
/// `,` or first braced group), when there is one: the scan leaves it out.
fn gated(trees: &[TokenTree], at: usize) -> Option<usize> {
    let attribute = |at: usize| {
        punct(trees, at, '#')
            && matches!(trees.get(at + 1), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket)
    };
    let Some(TokenTree::Group(first)) = trees.get(at + 1) else {
        return None;
    };
    let cfg = matches!(first.stream().into_iter().next(), Some(TokenTree::Ident(i)) if i == "cfg");
    if !attribute(at) || !cfg {
        return None;
    }
    let mut end = at + 2;
    while attribute(end) {
        end += 2;
    }
    while end < trees.len() {
        match &trees[end] {
            TokenTree::Punct(p) if matches!(p.as_char(), ';' | ',') => return Some(end + 1),
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => return Some(end + 1),
            _ => end += 1,
        }
    }
    Some(end)
}

/// `path`, written `depth` inline modules below the copy's module with the names `scope` binds,
/// as it reads from the copy's module; none where it stays inside the inline modules.
fn local(scope: &Scope, path: &[String], depth: usize) -> Option<Vec<String>> {
    match scope.get(&path[0]) {
        Some(Some(bound)) => Some(bound.iter().chain(&path[1..]).cloned().collect()),
        Some(None) => None,
        None => from_copy(path.to_vec(), depth),
    }
}

/// `path`, written `depth` inline modules below the copy's module, as it reads from that module;
/// none where it stays inside the inline modules.
fn from_copy(path: Vec<String>, depth: usize) -> Option<Vec<String>> {
    if depth == 0 || path[0] == "crate" {
        return Some(path);
    }
    let ups = path.iter().take_while(|s| *s == "super").count();
    match ups.cmp(&depth) {
        std::cmp::Ordering::Less => None,
        std::cmp::Ordering::Equal if path.len() > ups => Some(
            std::iter::once("self".to_owned())
                .chain(path[ups..].iter().cloned())
                .collect(),
        ),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(path[depth..].to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Cache, crate_mirror};
    use super::*;
    use quote::quote;
    use std::path::Path;
    use std::process::Command;

    /// The lower crate the moved items are in.
    const GAMEPLAY: &str = "
        pub mod api_values {
            pub trait TryIntoApi { fn api(&self) -> u32; }
            impl TryIntoApi for u8 { fn api(&self) -> u32 { u32::from(*self) } }
        }
        pub mod minecarts {
            #[derive(PartialEq)] pub enum CartKind { Rideable, Tnt }
            pub const SEAT: u32 = 2;
        }
        pub mod execution_role { #[derive(PartialEq)] pub enum ExecutionRole { Client } }
        pub mod rails { pub const GAUGE: u32 = 3; }
        pub mod depots { pub const DEPTH: u32 = 4; }
        pub mod tracks { pub const WIDTH: u32 = 5; }
        pub mod lanes { pub const LANE: u32 = 6; }
    ";

    /// The copied function, as written in the game: it reaches each moved item through an import
    /// the prebuilt game keeps private, from a game module's `use`, a body path (in a macro's
    /// arguments too), a name a glob brings, a glob and an alias of its own, and a public
    /// forwarding module.
    const RIDE: &str = "pub fn ride(value: u8) -> u32 {
        use super::TryIntoApi;
        use super::rails::*;
        use super::depots as yard;
        let kind = super::minecarts::CartKind::Rideable;
        let role = execution_role::ExecutionRole::Client;
        if matches!(kind, super::minecarts::CartKind::Rideable)
            && role == execution_role::ExecutionRole::Client
        {
            value.api() + super::minecarts::SEAT + GAUGE + yard::DEPTH
                + super::facade::carts::WIDTH + helper::lane()
        } else {
            0
        }
    }";

    /// A support module beside the copy: its paths leave it with `super`.
    const HELPER: &str = "pub mod helper { pub fn lane() -> u32 { super::super::lanes::LANE } }";

    /// A support module of `game`: a glob that makes `subtitles` a mirror, which must keep the
    /// rest of `game`'s grouped import of it; an import a disabled `cfg` leaves out, of a module the
    /// build does not have; a glob of `friends`, which two files declare under opposite `cfg`s;
    /// and an import of `item_refs`, one file and the fragment it `include!`s.
    const SUPPORT: &str = "pub(crate) mod support {
        #[cfg(any())]
        use crate::game::refs::ray_tests::*;
        use super::subtitles::*;
        use crate::game::friends::*;
        pub fn caption() -> super::Subtitles {
            let _ = LANE + super::subtitles() + invite(&super::Game) + super::item_refs::ITEM_LANE;
            super::Subtitles
        }
    }";

    /// `friends` with the `friends` feature, which imports a module only that build has.
    const FRIENDS: &str = "// game-module-cfg: feature = \"friends\"
        use super::Game;
        use super::screen::Panel;
        pub fn invite(_: &Game) -> u32 { Panel::SIZE }";

    /// `item_refs`, whose `read` is in the fragment `item_refs/read.rs` it includes.
    const ITEM_REFS: &str = "include!(\"item_refs/read.rs\");
        pub(crate) use gameplay::lanes::LANE as ITEM_LANE;
        pub fn owner() -> u32 { read() }";

    /// `friends` without it.
    const FRIENDS_OFF: &str = "// game-module-cfg: not(feature = \"friends\")
        // game-module-name: friends
        use super::Game;
        pub fn invite(_: &Game) -> u32 { 0 }";

    /// The game crate as the staging pass leaves it, items public and imports as written, into
    /// `dir`; and the patch crate's mirror of it with `RIDE`, `HELPER` and `SUPPORT` copied.
    fn mirrored(dir: &Path) -> TokenStream {
        let friends = dir.join("friends.rs");
        let friends_off = dir.join("friends_off.rs");
        std::fs::write(&friends, FRIENDS).unwrap();
        std::fs::write(&friends_off, FRIENDS_OFF).unwrap();
        // Declared as the game's build script declares its modules: out of syn's sight.
        let declare = |file: &Path, cfg: &str| {
            let file = file.to_string_lossy().replace('\\', "/");
            format!(
                "#[cfg({cfg})] #[path = {file:?}] pub mod friends;
"
            )
        };
        let item_refs = dir.join("item_refs.rs");
        let item_read = dir.join("item_refs").join("read.rs");
        std::fs::create_dir_all(dir.join("item_refs")).unwrap();
        std::fs::write(&item_refs, ITEM_REFS).unwrap();
        std::fs::write(&item_read, "pub fn read() -> u32 { 9 }").unwrap();
        std::fs::write(
            dir.join("game_modules.rs"),
            declare(&friends, "feature = \"friends\"")
                + &declare(&friends_off, "not(feature = \"friends\")")
                + &format!(
                    "#[path = {:?}] pub mod item_refs;
",
                    item_refs.to_string_lossy().replace('\\', "/")
                ),
        )
        .unwrap();
        let game = dir.join("lib.rs");
        std::fs::write(
            &game,
            format!(
                "pub mod game {{
                    include!(\"game_modules.rs\");
                    pub struct Game;
                    #[cfg(feature = \"friends\")]
                    pub mod screen {{
                        pub struct Panel;
                        impl Panel {{ pub const SIZE: u32 = 1; }}
                    }}
                    use self::api_values::TryIntoApi;
                    #[cfg(any())]
                    use self::quiet::LANE;
                    pub mod api_values {{ pub(crate) use gameplay::api_values::TryIntoApi; }}
                    pub mod minecarts {{ pub(in crate::game) use gameplay::minecarts::*; }}
                    pub mod execution_role {{
                        pub(super) use gameplay::execution_role::ExecutionRole;
                    }}
                    pub mod rails {{ pub(in crate::game) use gameplay::rails::*; }}
                    pub mod depots {{ pub(crate) use gameplay::depots::DEPTH; }}
                    pub mod tracks {{ pub(crate) use gameplay::tracks::WIDTH; }}
                    pub mod facade {{ pub use super::tracks as carts; }}
                    pub mod lanes {{ pub(crate) use gameplay::lanes::LANE; }}
                    pub mod quiet {{ pub(crate) use gameplay::lanes::LANE; }}
                    pub mod riding {{ use super::*; {RIDE} {HELPER} }}
                    use subtitles::{{Subtitles, subtitles}};
                    pub mod subtitles {{
                        pub struct Subtitles;
                        pub fn subtitles() -> u32 {{ 7 }}
                        pub(crate) use gameplay::lanes::LANE;
                    }}
                    pub mod refs {{
                        pub fn read() {{}}
                        #[cfg(test)]
                        pub mod ray_tests {{ use super::*; pub fn entity() {{}} }}
                    }}
                }}"
            ),
        )
        .unwrap();
        let mut source = Source {
            root: dir.to_owned(),
            loaded: BTreeSet::new(),
            files: BTreeMap::from([
                ("horizon_client".to_owned(), vec![game]),
                (
                    "horizon_client::game::friends".to_owned(),
                    vec![friends, friends_off],
                ),
                (
                    "horizon_client::game::item_refs".to_owned(),
                    vec![item_read, item_refs],
                ),
            ]),
            parsed: BTreeMap::new(),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        let mut tree = Tree::default();
        let game = tree.children.entry("game".into()).or_default();
        game.copies.push(SUPPORT.parse().unwrap());
        game.own.insert("support".into());
        let riding = game.children.entry("riding".into()).or_default();
        riding.copies.push(RIDE.parse().unwrap());
        riding.copies.push(HELPER.parse().unwrap());
        riding.own.extend(["ride".to_owned(), "helper".to_owned()]);
        crate_mirror(&mut source, "horizon_client", &mut tree, false, None).unwrap()
    }

    /// The modules of `items` that are mirrored (`pub mod x { … }`), by path below `prefix`.
    fn modules(items: &[Item], prefix: &str, out: &mut BTreeSet<String>) {
        for item in items {
            if let Item::Mod(module) = item
                && let Some((_, inner)) = &module.content
            {
                let path = format!("{prefix}::{}", module.ident);
                modules(inner, &path, out);
                out.insert(path);
            }
        }
    }

    #[test]
    fn a_copy_mirrors_the_modules_whose_imports_it_reaches() {
        let dir = tempfile::tempdir().unwrap();
        let file: syn::File = syn::parse2(mirrored(dir.path())).unwrap();
        let mut found = BTreeSet::new();
        modules(&file.items, "crate", &mut found);
        // `quiet` and `refs::ray_tests` are reached only by `use`s a disabled `cfg` leaves out.
        let expected = [
            "game",
            "game::api_values",
            "game::depots",
            "game::execution_role",
            "game::facade",
            "game::friends",
            "game::item_refs",
            "game::lanes",
            "game::minecarts",
            "game::rails",
            "game::riding",
            "game::riding::helper",
            "game::subtitles",
            "game::support",
            "game::tracks",
        ]
        .map(|m| format!("crate::{m}"));
        assert_eq!(found, BTreeSet::from(expected));
        // `game`'s grouped import keeps what does not collide with the `subtitles` mirror: the
        // struct, and the function that shares the module's name.
        let game = file
            .items
            .iter()
            .find_map(|item| match item {
                Item::Mod(module) if module.ident == "game" => module.content.as_ref(),
                _ => None,
            })
            .unwrap();
        let mut imported = Vec::new();
        for item in &game.1 {
            if let Item::Use(item) = item {
                let mut found = Vec::new();
                leaves(&item.tree, &mut Vec::new(), &mut found);
                imported.extend(found.into_iter().map(|leaf| leaf.path.join("::")));
            }
        }
        for name in ["subtitles::Subtitles", "subtitles::subtitles"] {
            assert!(imported.iter().any(|i| i == name), "{name} in {imported:?}");
        }
        // `friends` has a mirror per file, each under its file's `cfg` and with its items alone.
        let friends: Vec<String> = game
            .1
            .iter()
            .filter_map(|item| match item {
                Item::Mod(module) if module.ident == "friends" => {
                    Some(module.to_token_stream().to_string())
                }
                _ => None,
            })
            .collect();
        assert_eq!(friends.len(), 2, "{friends:?}");
        assert!(friends[0].starts_with("# [cfg (feature = \"friends\")]"));
        assert!(friends[0].contains("screen"));
        assert!(friends[1].starts_with("# [cfg (not (feature = \"friends\"))]"));
        assert!(!friends[1].contains("screen"));
        // `item_refs` and its fragment are one module: one mirror, with both files' items.
        let item_refs: Vec<String> = game
            .1
            .iter()
            .filter_map(|item| match item {
                Item::Mod(module) if module.ident == "item_refs" => {
                    Some(module.to_token_stream().to_string())
                }
                _ => None,
            })
            .collect();
        assert_eq!(item_refs.len(), 1, "{item_refs:?}");
        assert!(item_refs[0].starts_with("pub mod item_refs"));
        assert!(item_refs[0].contains("read") && item_refs[0].contains("owner"));
    }

    /// A mirror of a module the build may leave out keeps its `cfg`: an inline module's, and a
    /// game module file's (`// game-module-cfg:`).
    #[test]
    fn a_mirror_keeps_its_modules_cfg() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("lib.rs");
        let game = dir.path().join("game.rs");
        let bench = dir.path().join("bench.rs");
        std::fs::write(&root, "pub mod game;").unwrap();
        std::fs::write(
            &game,
            "#[cfg(feature = \"extra\")] pub mod gated { pub(crate) use std::rc::Rc; }",
        )
        .unwrap();
        std::fs::write(
            &bench,
            "// game-module-cfg: test\npub(crate) use std::sync::Arc;",
        )
        .unwrap();
        let mut source = Source {
            root: dir.path().to_owned(),
            loaded: BTreeSet::new(),
            files: BTreeMap::from([
                ("horizon_client".to_owned(), vec![root]),
                ("horizon_client::game".to_owned(), vec![game]),
                ("horizon_client::game::bench".to_owned(), vec![bench]),
            ]),
            parsed: BTreeMap::new(),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        let mut tree = Tree::default();
        let game = tree.children.entry("game".into()).or_default();
        game.children.entry("gated".into()).or_default();
        game.children.entry("bench".into()).or_default();
        let body = crate_mirror(&mut source, "horizon_client", &mut tree, false, None).unwrap();
        let file: syn::File = syn::parse2(body).unwrap();
        let Some(Item::Mod(game)) = file.items.iter().find(|i| matches!(i, Item::Mod(_))) else {
            panic!("no game mirror");
        };
        let guards: BTreeMap<String, String> = game
            .content
            .as_ref()
            .unwrap()
            .1
            .iter()
            .filter_map(|item| match item {
                Item::Mod(module) => Some((
                    module.ident.to_string(),
                    module
                        .attrs
                        .iter()
                        .map(|a| a.to_token_stream().to_string())
                        .collect::<Vec<_>>()
                        .join(" "),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(guards["gated"], "# [cfg (feature = \"extra\")]");
        assert_eq!(guards["bench"], "# [cfg (test)]");
    }

    /// The mirror compiles against a prebuilt game whose imports are private, with the
    /// `friends` feature and without, as the launcher passes the game's features to it.
    #[test]
    #[ignore = "daily"]
    fn the_patch_crate_compiles_against_the_games_private_imports() {
        let dir = tempfile::tempdir().unwrap();
        let rustc = |args: &[&str]| {
            let output = Command::new("rustc")
                .args(["--edition=2024", "--crate-type=rlib", "-L"])
                .arg(dir.path())
                .arg("--out-dir")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        let lower = dir.path().join("gameplay.rs");
        std::fs::write(&lower, GAMEPLAY).unwrap();
        rustc(&[lower.to_str().unwrap()]);
        let body = mirrored(dir.path());
        let game = dir.path().join("lib.rs");
        let patch = dir.path().join("patch.rs");
        std::fs::write(
            &patch,
            quote!(#![allow(unused_imports, dead_code)] #body).to_string(),
        )
        .unwrap();
        for features in [&[][..], &["--cfg", "feature=\"friends\""]] {
            let mut args = vec![
                "--crate-name=horizon_client",
                game.to_str().unwrap(),
                "--extern",
                "gameplay",
            ];
            args.extend(features);
            rustc(&args);
            let mut args = vec![
                "--crate-name=horizon_patched",
                patch.to_str().unwrap(),
                "--extern",
                "horizon_client",
                "--extern",
                "gameplay",
            ];
            args.extend(features);
            rustc(&args);
        }
    }

    #[test]
    fn copied_paths_are_read_as_their_module_reads_them() {
        let tokens: TokenStream = syn::parse_quote! {
            fn f() {
                use super::minecarts as carts;
                use super::rails::*;
                let a = carts::CartKind::Rideable;
                {
                    use super::depots as carts;
                    carts::DEPTH;
                }
                matches!(k, self::m::K::A);
                ::std::mem::drop(a);
                x.y::<u8>();
                $crate::game::f();
                $v::g();
                Vec::<u8>::new();
                #[cfg(any())]
                {
                    super::hidden::X;
                }
                #[cfg(any())]
                use super::gone::*;
            }
            #[cfg(test)]
            mod tests {
                use super::super::tested::*;
            }
            mod helper {
                use super::super::lanes as l;
                fn h() { l::LANE; super::sibling::X; self::inner::Y; }
            }
            macro_rules! m {
                () => { use $crate::game::bound::Thing; };
            }
        };
        let mut found = Vec::new();
        scan(tokens, 0, &Scope::new(), &mut found);
        let paths: Vec<(String, bool)> = found
            .iter()
            .map(|(path, glob)| (path.join("::"), *glob))
            .collect();
        let expect = |path: &str, glob| (path.to_owned(), glob);
        assert_eq!(
            paths,
            [
                expect("super::minecarts", false),
                expect("super::rails", true),
                expect("super::minecarts::CartKind::Rideable", false),
                expect("super::depots", false),
                expect("super::depots::DEPTH", false),
                expect("self::m::K::A", false),
                expect("crate::game::f", false),
                expect("super::lanes", false),
                expect("super::lanes::LANE", false),
                expect("self::sibling::X", false),
                expect("crate::game::bound::Thing", false),
            ]
        );
    }
}
