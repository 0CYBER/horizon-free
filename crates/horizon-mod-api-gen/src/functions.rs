//! The function index (`functions.json` in the SDK's build data): every function of the game's
//! source a `[[native.mixin]]` row may name, with where it is and whether function replacement can
//! take it. The staging pass writes it from the pristine source, with the same checks
//! [`crate::patch`] once ran on the player's machine on every build (a walk of the game's whole
//! source and a parse of every crate below it), so a mixin build reads only the files of the
//! modules its targets are in. Before a function is copied, its text is checked against the
//! index's hash: a source that is not the build's is refused by name, never walked.
use crate::patch::{Found, Source, TargetPath, plain, replaceable, target_path};
use quote::ToTokens;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use syn::spanned::Spanned;
use syn::{ImplItem, Item};

/// The index's file name in the build data.
pub const FILE: &str = point_shape::build_data::FUNCTIONS;

/// The function index of one game build.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionIndex {
    /// The crates a function may be copied from, by the name a target starts with.
    pub crates: BTreeMap<String, CrateModules>,
    /// The functions function replacement can take, by [`key`].
    pub functions: BTreeMap<String, Function>,
    /// The functions it can't, by [`key`], with the refusal a row naming one gets.
    pub refused: BTreeMap<String, Refused>,
}

/// One crate's modules, as a copy's mirror reads them. Every crate of the source is listed, so a
/// target naming none of them is told so; only the game crate and the crates with
/// `#[replaceable]` functions list their modules.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateModules {
    /// Its library's name (`[lib] name`), which the patch crate names it by.
    pub library: String,
    /// Each module's files.
    pub modules: BTreeMap<String, Vec<ModuleFile>>,
}

/// A file a copy may read: the patch step checks it is the build's before reading it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleFile {
    /// Relative to the source root, `/`-separated.
    pub path: String,
    /// The SHA-256 of the file's bytes, hex.
    pub sha256: String,
}

/// A function that can take a replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    /// The module its file is, as [`CrateModules::modules`] keys it (an inline module's function is
    /// in a module below it).
    pub module: String,
    /// Its file, relative to the source root, `/`-separated.
    pub file: String,
    /// Its bytes in the file, `[start, end)`: attributes, signature and body.
    pub span: [usize; 2],
    /// The hash of its signature, as the other build-data files hash theirs.
    pub signature_hash: String,
    /// The SHA-256 of the span's bytes, hex.
    pub sha256: String,
}

/// Why a function can't take a replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refused {
    pub code: Code,
    pub why: String,
}

/// A refusal's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Code {
    #[serde(rename = "E_MIXIN_NOT_REPLACEABLE")]
    NotReplaceable,
    #[serde(rename = "E_MIXIN_TARGET_AMBIGUOUS")]
    Ambiguous,
}

impl Code {
    pub fn name(self) -> &'static str {
        match self {
            Code::NotReplaceable => "E_MIXIN_NOT_REPLACEABLE",
            Code::Ambiguous => "E_MIXIN_TARGET_AMBIGUOUS",
        }
    }
}

/// A target as the index keys it: no generic arguments or spaces, and a trait method's trait by
/// its last segment (`m::<GameLevel as LevelAccess>::block_state`), as the copy finds it.
pub fn key(target: &str) -> String {
    match target_path(target) {
        None => plain(target),
        Some(TargetPath {
            qualified: Some((module, ty, tr)),
            name,
            ..
        }) => format!("{}::<{ty} as {tr}>::{name}", plain(module)),
        Some(TargetPath { front, name, .. }) => format!("{}::{name}", plain(front)),
    }
}

/// `relative`, an index's path, under `root`: a path that would leave it is refused.
pub(crate) fn source_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!(
            "{FILE} names `{relative}`, which is not a path inside the game's source"
        ));
    }
    Ok(root.join(path))
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl FunctionIndex {
    /// Whether `target` can be copied out of the source at `root`: listed as replaceable, and its
    /// span there is the indexed text. Otherwise its refusal, `(code, why)`.
    ///
    /// Only the span is checked here; every other file the copy reads is checked against its
    /// [`ModuleFile::sha256`] as it is read.
    pub fn check(&self, root: &Path, target: &str) -> Result<(), (&'static str, String)> {
        let key = key(target);
        let krate = key.split("::").next().unwrap_or_default();
        if !self.crates.contains_key(krate) {
            return Err((
                "E_MIXIN_TARGET_MISSING",
                format!("no crate `{krate}` in the game's source"),
            ));
        }
        if let Some(refused) = self.refused.get(&key) {
            return Err((refused.code.name(), refused.why.clone()));
        }
        let Some(function) = self.functions.get(&key) else {
            return Err(if krate == "horizon_client" {
                (
                    "E_MIXIN_TARGET_MISSING",
                    format!("no function `{key}` in this game build's function index ({FILE})"),
                )
            } else {
                (
                    "E_MIXIN_NOT_REPLACEABLE",
                    crate::replace::UNMARKED.to_owned(),
                )
            });
        };
        let stale = |why: String| ("E_MIXIN_SOURCE", why);
        let path = source_path(root, &function.file).map_err(stale)?;
        let bytes = std::fs::read(&path).map_err(|e| stale(format!("{}: {e}", path.display())))?;
        let [start, end] = function.span;
        let text = bytes.get(start..end).ok_or_else(|| {
            stale(format!(
                "{} has no bytes {start}..{end}: it is not the source this game build's {FILE} describes",
                function.file
            ))
        })?;
        if sha256(text) != function.sha256 {
            return Err(stale(format!(
                "{} bytes {start}..{end} differ from this game build's {FILE}: the source is not the build's",
                function.file
            )));
        }
        Ok(())
    }

    /// The index as its reader may trust it, or why not: every crate and library is a Rust
    /// name and every module a path of one, as the copy writes them into Rust code, and each
    /// function is in a module the index lists. A downloaded index is checked here once, so
    /// nothing in it reaches the copy unchecked.
    pub fn checked(self) -> Result<Self, String> {
        let name = |text: &str| syn::parse_str::<syn::Ident>(text).is_ok();
        let path = |text: &str| text.split("::").all(name);
        for (krate, facts) in &self.crates {
            if !name(krate) || !name(&facts.library) {
                return Err(format!(
                    "{FILE}: crate `{krate}` or its library `{}` is not a Rust name",
                    facts.library
                ));
            }
            if let Some(module) = facts
                .modules
                .keys()
                .find(|module| !path(module) || module.split("::").next() != Some(krate))
            {
                return Err(format!(
                    "{FILE}: `{module}` is not a module path of crate `{krate}`"
                ));
            }
        }
        if let Some((key, function)) = self.functions.iter().find(|(_, function)| {
            let krate = function.module.split("::").next().unwrap_or_default();
            !self
                .crates
                .get(krate)
                .is_some_and(|facts| facts.modules.contains_key(&function.module))
        }) {
            return Err(format!(
                "{FILE}: function `{key}` is in `{}`, which is not a module the index lists",
                function.module
            ));
        }
        Ok(self)
    }

    /// The function `target` names, read at its indexed span: the very item the index's verdict
    /// is about, never one looked up again by name. Call it once [`FunctionIndex::check`] passed.
    pub(crate) fn found(
        &self,
        source: &mut Source,
        target: &str,
    ) -> Result<Found, (&'static str, String)> {
        let key = key(target);
        let stale = |why: String| ("E_MIXIN_SOURCE", why);
        let function = self
            .functions
            .get(&key)
            .ok_or_else(|| stale(format!("no function `{key}` in {FILE}")))?;
        let path = source_path(source.root(), &function.file).map_err(stale)?;
        let text = source.read(&path).map_err(stale)?;
        let items = syn::parse_file(&text)
            .map_err(|e| stale(format!("{}: {e}", path.display())))?
            .items;
        let lines = line_starts(&text);
        let file = File {
            relative: &function.file,
            module: &function.module,
            text: &text,
            lines: &lines,
        };
        let mut candidates = BTreeMap::new();
        collect(&function.module, &items, &file, &mut candidates);
        let [start, end] = function.span;
        candidates
            .remove(&key)
            .into_iter()
            .flatten()
            .find(|candidate| candidate.function.span == function.span)
            .map(|candidate| candidate.found)
            .ok_or_else(|| {
                stale(format!(
                    "{} has no function `{key}` at bytes {start}..{end}, where this game build's {FILE} has it",
                    function.file
                ))
            })
    }

    /// Refuses each listed function, except a published loop's generic one (`generic`, keys),
    /// that the staging pass gave no replacement slot (`slots`, keys): it has no slot to take a
    /// copy.
    pub fn retain_slotted(&mut self, slots: &BTreeSet<String>, generic: &BTreeSet<String>) {
        let unslotted: Vec<String> = self
            .functions
            .keys()
            .filter(|key| !slots.contains(*key) && !generic.contains(*key))
            .cloned()
            .collect();
        for key in unslotted {
            self.functions.remove(&key);
            self.refused.insert(
                key,
                Refused {
                    code: Code::NotReplaceable,
                    why: "the game build gives it no replacement slot".into(),
                },
            );
        }
    }
}

/// The index of the pristine source at `root`, whose published loops' generic functions are
/// `generic` (keys). Each function's verdict is [`replaceable`]'s, the check a mixin build ran
/// before the index; [`FunctionIndex::retain_slotted`] then drops what staging gave no slot.
pub fn index(root: &Path, generic: &BTreeSet<String>) -> Result<FunctionIndex, String> {
    index_of(root, generic, None)
}

/// [`index`], listing only the functions whose keys are in `only`, when given.
pub(crate) fn index_of(
    root: &Path,
    generic: &BTreeSet<String>,
    only: Option<&BTreeSet<String>>,
) -> Result<FunctionIndex, String> {
    let mut source = Source::new(root)?;
    // Below the game crate, only the functions marked `#[replaceable]` take a replacement.
    let marked: BTreeSet<String> = crate::replace::marked(root)?
        .into_iter()
        .map(|(_, path)| key(&path))
        .collect();
    let lower: BTreeSet<&str> = marked
        .iter()
        .filter_map(|path| path.split("::").next())
        .filter(|krate| *krate != "horizon_client")
        .collect();
    for krate in lower {
        if !source.load(krate) {
            return Err(format!(
                "{krate}: it has #[horizon_mod_api_macros::replaceable] functions, but its sources can't be read"
            ));
        }
    }
    let mut index = FunctionIndex::default();
    for (module, paths) in &source.files {
        let krate = module.split("::").next().unwrap_or_default();
        let facts = index
            .crates
            .entry(krate.to_owned())
            .or_insert_with(|| CrateModules {
                library: source
                    .libraries
                    .get(krate)
                    .cloned()
                    .unwrap_or_else(|| krate.to_owned()),
                modules: BTreeMap::new(),
            });
        let mut listed = Vec::new();
        for path in paths {
            // A `#[path]` leads through `..`: the index names the file as the archive holds it.
            let inside = path.strip_prefix(root).ok().and_then(|inside| {
                let mut parts: Vec<String> = Vec::new();
                for component in inside.components() {
                    match component {
                        Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
                        Component::ParentDir => drop(parts.pop()?),
                        _ => {}
                    }
                }
                Some(parts.join("/"))
            });
            let relative = inside.ok_or_else(|| {
                format!(
                    "{}: outside the source at {}",
                    path.display(),
                    root.display()
                )
            })?;
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            listed.push(ModuleFile {
                path: relative,
                sha256: sha256(&bytes),
            });
        }
        facts.modules.insert(module.clone(), listed);
    }
    // The other crates, by name only: a target in one is told it is not marked, not that the
    // crate does not exist.
    let crates = root.join("crates");
    for entry in std::fs::read_dir(&crates).map_err(|e| format!("{}: {e}", crates.display()))? {
        let package = entry
            .map_err(|e| format!("{}: {e}", crates.display()))?
            .path();
        let Some(folder) = package.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !package.join("Cargo.toml").is_file() {
            continue;
        }
        let krate = folder.replace('-', "_");
        if let std::collections::btree_map::Entry::Vacant(slot) = index.crates.entry(krate) {
            slot.insert(CrateModules {
                library: crate::replace::library_name(&package)?,
                modules: BTreeMap::new(),
            });
        }
    }
    // Every function a target can name, by key, with its file and span.
    let mut candidates: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
    let files: Vec<(String, String, PathBuf)> = index
        .crates
        .values()
        .flat_map(|facts| {
            facts.modules.iter().flat_map(|(module, files)| {
                files
                    .iter()
                    .map(|file| (module.clone(), file.path.clone(), root.join(&file.path)))
            })
        })
        .collect();
    for (module, relative, path) in files {
        let prefix = format!("{module}::");
        if only.is_some_and(|only| !only.iter().any(|key| key.starts_with(&prefix))) {
            continue;
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let items = syn::parse_file(&text)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .items;
        let lines = line_starts(&text);
        let file = File {
            relative: &relative,
            module: &module,
            text: &text,
            lines: &lines,
        };
        collect(&module, &items, &file, &mut candidates);
    }
    for (key, mut found) in candidates {
        let krate = key.split("::").next().unwrap_or_default();
        if only.is_some_and(|only| !only.contains(&key))
            || (krate != "horizon_client" && !marked.contains(&key))
        {
            continue;
        }
        if found.len() > 1 {
            index.refused.insert(
                key,
                Refused {
                    code: Code::Ambiguous,
                    why: "several functions have this path (a cfg-gated pair, for one)".into(),
                },
            );
            continue;
        }
        let candidate = found.pop().expect("one");
        match replaceable(&candidate.found, &mut source, generic.contains(&key)) {
            Ok(()) => {
                index.functions.insert(key, candidate.function);
            }
            Err(why) => {
                index.refused.insert(
                    key,
                    Refused {
                        code: Code::NotReplaceable,
                        why,
                    },
                );
            }
        }
    }
    Ok(index)
}

/// A function found in a file, before its verdict.
struct Candidate {
    found: Found,
    function: Function,
}

/// Where each line of `text` starts.
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// A file being indexed.
struct File<'a> {
    relative: &'a str,
    /// The module the file is.
    module: &'a str,
    text: &'a str,
    lines: &'a [usize],
}

impl File<'_> {
    fn function(&self, span: proc_macro2::Span, sig: &syn::Signature) -> Function {
        let start = crate::source::offset(self.text, self.lines, span.start());
        let end = crate::source::offset(self.text, self.lines, span.end());
        Function {
            module: self.module.to_owned(),
            file: self.relative.to_owned(),
            span: [start, end],
            signature_hash: crate::internals::signature_hash(&sig.to_token_stream().to_string()),
            sha256: sha256(&self.text.as_bytes()[start..end]),
        }
    }
}

/// The functions of `items`, module `module`, by the key a target naming each has: free
/// functions and methods of every `impl`, inline modules included (test modules left out).
fn collect(
    module: &str,
    items: &[Item],
    file: &File<'_>,
    out: &mut BTreeMap<String, Vec<Candidate>>,
) {
    for item in items {
        match item {
            Item::Fn(function) => {
                out.entry(format!("{module}::{}", function.sig.ident))
                    .or_default()
                    .push(Candidate {
                        found: Found {
                            module: module.to_owned(),
                            owner: None,
                            function: syn::ImplItemFn {
                                attrs: function.attrs.clone(),
                                vis: function.vis.clone(),
                                defaultness: None,
                                sig: function.sig.clone(),
                                block: (*function.block).clone(),
                            },
                        },
                        function: file.function(function.span(), &function.sig),
                    });
            }
            Item::Impl(block) => {
                // The header and its associated types and constants: the verdict reads the
                // `impl`'s attributes, generics, trait and type, and a trait method's copy names
                // the associated items through the type; never the other methods.
                let header = syn::ItemImpl {
                    attrs: block.attrs.clone(),
                    defaultness: block.defaultness,
                    unsafety: block.unsafety,
                    impl_token: block.impl_token,
                    generics: block.generics.clone(),
                    trait_: block.trait_.clone(),
                    self_ty: block.self_ty.clone(),
                    brace_token: block.brace_token,
                    items: block
                        .items
                        .iter()
                        .filter(|item| !matches!(item, ImplItem::Fn(_)))
                        .cloned()
                        .collect(),
                };
                let owner = plain(&block.self_ty.to_token_stream().to_string());
                let scope = match &block.trait_ {
                    Some((_, path, _)) => {
                        let last = path
                            .segments
                            .last()
                            .map(|s| s.ident.to_string())
                            .unwrap_or_default();
                        format!("{module}::<{owner} as {last}>")
                    }
                    None => format!("{module}::{owner}"),
                };
                for member in &block.items {
                    if let ImplItem::Fn(method) = member {
                        out.entry(format!("{scope}::{}", method.sig.ident))
                            .or_default()
                            .push(Candidate {
                                found: Found {
                                    module: module.to_owned(),
                                    owner: Some(header.clone()),
                                    function: method.clone(),
                                },
                                function: file.function(method.span(), &method.sig),
                            });
                    }
                }
            }
            Item::Mod(inline) if !mod_source::modules::test_only(&inline.attrs) => {
                if let Some((_, inner)) = &inline.content {
                    collect(&format!("{module}::{}", inline.ident), inner, file, out);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
