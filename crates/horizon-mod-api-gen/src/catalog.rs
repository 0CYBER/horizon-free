//! The hook catalog as typed data: the declaration files under `crates/horizon-mod-vocab/src/hooks`, read once.
//!
//! The loader here is the only code that reads the declarations. Every generated output
//! (the `Hooks` dispatch, the Host traits, the game's entry points, the rustdoc, the mixin hook
//! names) reads these types.

#[cfg(test)]
mod collect_validation_tests;

use quote::ToTokens;
use serde::Deserialize;
use std::{collections::HashSet, fmt, fs, path::Path};

/// Which side of the connection a hook or helper runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Server,
    Client,
    Both,
}

/// What a hook's listeners may do with its event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Contract {
    /// Told about a moment; nothing comes back.
    Listen,
    /// May veto it; once cancelled, later listeners run only if they receive cancelled events,
    /// and those may undo it.
    Cancel,
    /// Every listener adds to, changes or removes from one output.
    Collect,
    /// The first listener to answer sets the output.
    Replace,
}

/// The host a hook takes when its receiver does not say which.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostAccess {
    /// The shared read-only `Host`.
    Read,
    /// A level host, for a receiver that is a level but not `GameLevel`.
    Level,
}

/// A hook fired in a load phase, which takes that phase's context instead of a host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseContext {
    Registration,
    DataLoad,
    /// A hook fired inside a menu, recipe or player-list loop of one side.
    Loop,
    /// A hook fired while a feature generates, on a worldgen worker or the server thread.
    Worldgen,
}

/// The kind of thing whose Parts a scoped hook calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartKind {
    Block,
    BlockEntity,
    Item,
    Menu,
    Fluid,
    Entity,
}

/// A scoped hook: a Part method, called on the Parts attached to what `key` names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartScope {
    pub kind: PartKind,
    /// The hook argument naming the thing whose Parts are called.
    pub key: String,
}

/// The names a hook has in other mod APIs, for the rustdoc and for finding gaps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counterparts {
    pub neoforge: Vec<String>,
    pub fabric: Vec<String>,
    pub paper: Vec<String>,
}

/// Count admission for stack-list outputs. Trimming is deferred to placement.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StackCount {
    Trim,
}

/// One hook point: a moment the game tells mods about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hook {
    pub name: String,
    /// The rustdoc of its trait method.
    pub doc: String,
    /// The prose of `docs/modding/src/reference/hook-notes.md` for this hook, heading included.
    pub notes: Option<String>,
    pub side: Side,
    /// The game function that fires it.
    pub target: String,
    pub contract: Contract,
    /// The event field a cancel, collect or replace hook writes; `None` for a listener.
    pub out: Option<String>,
    /// A replace hook whose output also names the mod that answered, or a collect hook whose
    /// entries each name the mod that pushed it.
    pub answered_by_mod: bool,
    /// A listener about one mod's own things: the event field naming that mod, whose
    /// subscription alone is told.
    pub owner: Option<String>,
    /// Validate a collector's edits at the per-listener ownership boundary.
    pub collect_validator: Option<String>,
    /// A global cancel hook whose dispatch runs every listener inside `catch_unwind`: one that
    /// unwinds is switched off for this hook for the session, and the outputs it had written roll
    /// back to what the game requested (#2414 ruling).
    pub isolate: bool,
    pub scope: Option<PartScope>,
    pub host: Option<HostAccess>,
    pub context: Option<PhaseContext>,
    pub counterparts: Counterparts,
    /// The contract version an injection's `hook_version` names.
    pub version: u64,
    /// Whether the NeoForge counterpart event is cancellable, written by the
    /// stamper from the counterpart's javadoc; documentation metadata, not the
    /// contract (a hook that listens can still name a cancellable event).
    pub cancellable: Option<bool>,
    /// The parameters of a hook whose game target takes them as one generated `<Name>Call`
    /// (name and type, in order, without a `Ticks` parameter); empty for a flat target.
    pub call: Vec<(String, String)>,
    /// Parameter retained in a decision-scoped guest slot instead of copied per ask.
    pub prepared: Option<String>,
    /// Admission policy for the mutable stack-list output; placement owns trimming.
    pub stack_count: Option<StackCount>,
}

/// A drawing surface of the game's that drawing hooks get as a trait object (the catalog's
/// `draw_contexts`): a hook argument of the surface's type is the trait object, declared in
/// `host/draw.rs`, in the hook's event.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DrawContext {
    /// The game's type (`crate::hud::HudBatch`).
    pub surface: String,
    /// The trait mods draw through.
    #[serde(rename = "trait")]
    pub trait_name: String,
}

/// Why the catalog was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum CatalogError {
    Read { path: String, error: String },
    Declaration { path: String, error: String },
    NamelessHook,
    DuplicateHook(String),
    InvalidHook { hook: String, error: String },
    MissingDoc(String),
    Counterpart { hook: String, key: &'static str },
    InvalidContract(String),
    AnsweredNeedsGlobalOutput(String),
    OwnerNeedsGlobalListener(String),
    ScopeNeedsKey(String),
    NoHooks,
    Retired(String),
    InvalidModule(String),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, error } => write!(f, "{path}: {error}"),
            Self::Declaration { path, error } => write!(f, "{path}: {error}"),
            Self::Retired(path) => write!(
                f,
                "{path}: hooks are declared in Rust now, in {DECLARATIONS}/<area>.rs; move the entry there as a \
                 `#[hook(...)]` declaration (the old keys are the attribute's arguments) and delete this file"
            ),
            Self::InvalidModule(name) => write!(f, "invalid hook declaration filename {name}"),
            Self::NamelessHook => f.write_str("hook name missing"),
            Self::DuplicateHook(name) => write!(f, "duplicate hook {name}"),
            Self::InvalidHook { hook, error } => write!(f, "hook {hook}: {error}"),
            Self::MissingDoc(name) => write!(
                f,
                "hook {name}: doc missing: add `///` lines, the rustdoc of its trait method"
            ),
            Self::Counterpart { hook, key } => {
                write!(f, "hook {hook}: {key} must be a string or list of strings")
            }
            Self::InvalidContract(name) => write!(f, "invalid contract for {name}"),
            Self::AnsweredNeedsGlobalOutput(name) => {
                write!(
                    f,
                    "{name}: answered_by needs a global replace or collect hook"
                )
            }
            Self::OwnerNeedsGlobalListener(name) => {
                write!(f, "{name}: owner needs a global listener")
            }
            Self::ScopeNeedsKey(name) => write!(f, "{name} needs both scope and key, or neither"),
            Self::NoHooks => f.write_str("no native hooks"),
        }
    }
}

impl std::error::Error for CatalogError {}

impl From<CatalogError> for String {
    fn from(error: CatalogError) -> Self {
        error.to_string()
    }
}

/// A counterpart name or a list of them.
#[derive(Deserialize)]
#[serde(untagged)]
enum Names {
    One(String),
    Many(Vec<String>),
}

fn counterparts(
    hook: &str,
    key: &'static str,
    names: Option<serde_json::Value>,
) -> Result<Vec<String>, CatalogError> {
    let invalid = || CatalogError::Counterpart {
        hook: hook.to_owned(),
        key,
    };
    match names {
        None => Ok(Vec::new()),
        Some(value) => match serde_json::from_value(value).map_err(|_| invalid())? {
            Names::One(name) => Ok(vec![name]),
            Names::Many(names) => Ok(names),
        },
    }
}

/// A hook entry as the JSON spells it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookEntry {
    #[serde(rename = "name")]
    _name: String,
    doc: Option<String>,
    notes: Option<String>,
    side: Side,
    target: String,
    mod_api: ModApiEntry,
    version: Option<u64>,
    cancellable: Option<bool>,
    host: Option<HostAccess>,
    context: Option<PhaseContext>,
    #[serde(default)]
    call: Vec<(String, String)>,
    /// Only `prepared = "stack"` on `can_equip` is supported; reject at parse time.
    prepared: Option<String>,
    stack_count: Option<StackCount>,
    neoforge: Option<serde_json::Value>,
    fabric: Option<serde_json::Value>,
    paper: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModApiEntry {
    contract: Contract,
    out: Option<String>,
    scope: Option<PartKind>,
    key: Option<String>,
    answered_by: Option<AnsweredBy>,
    owner: Option<String>,
    collect_validator: Option<String>,
    isolate: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum AnsweredBy {
    Mod,
}

impl Hook {
    fn parse(value: serde_json::Value) -> Result<Self, CatalogError> {
        let name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or(CatalogError::NamelessHook)?
            .to_owned();
        let entry: HookEntry =
            serde_json::from_value(value).map_err(|error| CatalogError::InvalidHook {
                hook: name.clone(),
                error: error.to_string(),
            })?;
        // This transport is implemented only for can_equip's stack. Validate here,
        // before either host or guest generation can accept a different hook.
        if let Some(parameter) = &entry.prepared
            && (name != "can_equip" || parameter != "stack")
        {
            return Err(CatalogError::InvalidHook {
                hook: name,
                error: "prepared is supported only as prepared = \"stack\" on can_equip".into(),
            });
        }
        let doc = entry
            .doc
            .filter(|doc| !doc.trim().is_empty())
            .ok_or_else(|| CatalogError::MissingDoc(name.clone()))?;
        let api = entry.mod_api;
        let scope = match (api.scope, api.key) {
            (None, None) => None,
            (Some(kind), Some(key)) => Some(PartScope { kind, key }),
            _ => return Err(CatalogError::ScopeNeedsKey(name)),
        };
        if matches!(
            (api.contract, &api.out),
            (Contract::Listen, Some(_))
                | (
                    Contract::Cancel | Contract::Collect | Contract::Replace,
                    None
                )
        ) {
            return Err(CatalogError::InvalidContract(name));
        }
        if entry.stack_count.is_some() {
            if api.contract != Contract::Collect || api.out.is_none() || api.answered_by.is_some() {
                return Err(CatalogError::InvalidHook {
                    hook: name,
                    error: "stack_count requires a mutable Vec<ItemStack> collect output".into(),
                });
            }
            if !entry.call.is_empty() {
                let output = entry
                    .call
                    .iter()
                    .find(|(parameter, _)| Some(parameter) == api.out.as_ref())
                    .and_then(|(_, ty)| syn::parse_str::<syn::Type>(ty).ok());
                validate_stack_count_output(&name, output.as_ref())?;
            }
        }
        let answered_by_mod = api.answered_by.is_some();
        if answered_by_mod
            && (!matches!(api.contract, Contract::Replace | Contract::Collect) || scope.is_some())
        {
            return Err(CatalogError::AnsweredNeedsGlobalOutput(name));
        }
        if api.owner.is_some() && (api.contract != Contract::Listen || scope.is_some()) {
            return Err(CatalogError::OwnerNeedsGlobalListener(name));
        }
        // A validator checks the entries a mod edits in place; an answered collector's mods
        // only push into their own batch.
        if let Some(validator) = &api.collect_validator
            && (api.contract != Contract::Collect
                || scope.is_some()
                || answered_by_mod
                || syn::parse_str::<syn::Path>(validator).is_err())
        {
            return Err(CatalogError::InvalidContract(name));
        }
        // Isolation is the cancel loop's own: a listener that unwinds rolls the outputs back
        // to the requested ones and is skipped for the hook for the session (#2414 ruling).
        let isolate = api.isolate.unwrap_or(false);
        if isolate && (api.contract != Contract::Cancel || scope.is_some()) {
            return Err(CatalogError::InvalidHook {
                hook: name.clone(),
                error: "isolate is for a global cancel hook".into(),
            });
        }
        let counterparts = Counterparts {
            neoforge: counterparts(&name, "neoforge", entry.neoforge)?,
            fabric: counterparts(&name, "fabric", entry.fabric)?,
            paper: counterparts(&name, "paper", entry.paper)?,
        };
        Ok(Self {
            name,
            doc,
            notes: entry.notes,
            side: entry.side,
            target: entry.target,
            contract: api.contract,
            out: api.out,
            answered_by_mod,
            owner: api.owner,
            collect_validator: api.collect_validator,
            isolate,
            scope,
            host: entry.host,
            context: entry.context,
            counterparts,
            version: entry.version.unwrap_or(1),
            cancellable: entry.cancellable,
            call: entry.call,
            prepared: entry.prepared,
            stack_count: entry.stack_count,
        })
    }
}

/// Validate a declared or resolved native output at the catalog boundary.
pub(crate) fn validate_stack_count_output(
    name: &str,
    output: Option<&syn::Type>,
) -> Result<(), CatalogError> {
    let valid = (|| {
        let syn::Type::Reference(reference) = output? else {
            return None;
        };
        reference.mutability?;
        let syn::Type::Path(path) = &*reference.elem else {
            return None;
        };
        let vec = path.path.segments.last()?;
        if vec.ident != "Vec" {
            return None;
        }
        let syn::PathArguments::AngleBracketed(args) = &vec.arguments else {
            return None;
        };
        if args.args.len() != 1 {
            return None;
        }
        let syn::GenericArgument::Type(syn::Type::Path(item)) = args.args.first()? else {
            return None;
        };
        (item.path.segments.last()?.ident == "ItemStack").then_some(())
    })()
    .is_some();
    if valid {
        Ok(())
    } else {
        Err(CatalogError::InvalidHook {
            hook: name.into(),
            error: "stack_count requires a mutable Vec<ItemStack> collect output".into(),
        })
    }
}

/// Everything the generator reads from the catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    /// In declaration order: base first, then files by name, each file's declarations in order.
    pub hooks: Vec<Hook>,
    pub draw_contexts: Vec<DrawContext>,
    /// Prose that belongs to no one hook, from the `Notes` trait's `#[notes]`.
    pub general_notes: Option<String>,
}

/// Where the declarations live, relative to the checkout root.
pub const DECLARATIONS: &str = "crates/horizon-mod-vocab/src/hooks";

/// What the catalog was declared in before the declarations: a file here is refused, never ignored.
pub(crate) const RETIRED: &[&str] = &["native-hooks.json", "tools/native-hook-fragments"];

/// A declaration's inferred target, resolved to the definition's module by `Catalog::load`.
/// Parsing declarations alone does not require engine sources (render-record generation).
fn default_target(name: &str, area: &str) -> String {
    format!("horizon_client::game::native_hooks::{area}::Game<'_>::native_{name}")
}

/// An attribute argument as the entry structs read it.
fn argument(expr: &syn::Expr) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    match expr {
        syn::Expr::Lit(syn::ExprLit { lit, .. }) => match lit {
            syn::Lit::Str(text) => Ok(Value::String(text.value())),
            syn::Lit::Bool(flag) => Ok(Value::Bool(flag.value)),
            syn::Lit::Int(number) => number
                .base10_parse::<u64>()
                .map(Value::from)
                .map_err(|e| e.to_string()),
            _ => Err("expected a string, a bool or an integer".into()),
        },
        syn::Expr::Array(array) => array.elems.iter().map(argument).collect(),
        _ => Err("expected a literal or an array of literals".into()),
    }
}

/// The `key = value` arguments of the attribute named `attribute` on a declaration, as the JSON
/// object the entry structs deserialize; `None` when it carries no such attribute.
fn arguments(
    attrs: &[syn::Attribute],
    attribute: &str,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    let Some(attr) = attrs.iter().find(|a| a.path().is_ident(attribute)) else {
        return Ok(None);
    };
    let mut map = serde_json::Map::new();
    attr.parse_nested_meta(|meta| {
        let key = meta
            .path
            .get_ident()
            .ok_or_else(|| meta.error("expected a key"))?
            .to_string();
        let value = argument(&meta.value()?.parse()?).map_err(|e| meta.error(e))?;
        map.insert(key, value);
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    Ok(Some(map))
}

/// The `#[name = "..."]` string attributes on a declaration, in order, joined by a blank line.
fn string_attribute(attrs: &[syn::Attribute], name: &str) -> Result<Option<String>, String> {
    let mut values = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident(name)) {
        let syn::Meta::NameValue(syn::MetaNameValue {
            value:
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(text),
                    ..
                }),
            ..
        }) = &attr.meta
        else {
            return Err(format!("#[{name}] takes a string"));
        };
        values.push(text.value());
    }
    Ok((!values.is_empty()).then(|| values.join("\n\n")))
}

/// The rustdoc of a declaration: its `///` lines, one leading space off each, joined by newlines.
fn doc(attrs: &[syn::Attribute]) -> Option<String> {
    let lines: Vec<String> = attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(syn::MetaNameValue {
                path,
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(text),
                        ..
                    }),
                ..
            }) if path.is_ident("doc") => {
                let text = text.value();
                Some(text.strip_prefix(' ').unwrap_or(&text).to_owned())
            }
            _ => None,
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The declarations of the three kinds the files hold, as entry objects in file order.
#[derive(Default)]
struct Declared {
    hooks: Vec<serde_json::Value>,
    draw_contexts: Vec<serde_json::Value>,
    general_notes: Vec<String>,
}

impl Declared {
    /// Reads one declaration file: its `Hooks`, `DrawContexts` and `Notes` traits.
    fn read(&mut self, path: &str, text: &str) -> Result<(), CatalogError> {
        let invalid = |error: String| CatalogError::Declaration {
            path: path.to_owned(),
            error,
        };
        let area = Path::new(path)
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("declaration file has no area name".into()))?;
        let file = syn::parse_file(text).map_err(|e| invalid(e.to_string()))?;
        for item in file.items {
            let syn::Item::Trait(declarations) = item else {
                continue;
            };
            let kind = declarations.ident.to_string();
            if kind == "Helpers" {
                return Err(invalid(
                    "helpers are hand-written host traits now: declare the method in `crates/horizon-mod-vocab/src/host/<area>.rs`"
                        .into(),
                ));
            }
            for member in declarations.items {
                let syn::TraitItem::Fn(function) = member else {
                    return Err(invalid(format!(
                        "{kind} holds only declarations: fn name();"
                    )));
                };
                let name = function.sig.ident.to_string();
                if kind == "Notes" {
                    if let Some(notes) = string_attribute(&function.attrs, "notes")
                        .map_err(|e| invalid(format!("{name}: {e}")))?
                    {
                        self.general_notes.push(notes);
                    }
                    continue;
                }
                let attribute = match kind.as_str() {
                    "Hooks" => "hook",
                    "DrawContexts" => "draw_context",
                    _ => continue,
                };
                let mut entry = arguments(&function.attrs, attribute)
                    .map_err(|e| invalid(format!("{name}: {e}")))?
                    .ok_or_else(|| {
                        invalid(format!("{name}: needs a #[{attribute}(...)] attribute"))
                    })?;
                entry
                    .entry("target")
                    .or_insert_with(|| default_target(&name, area).into());
                if let Some(notes) = string_attribute(&function.attrs, "notes")
                    .map_err(|e| invalid(format!("{name}: {e}")))?
                {
                    entry.insert("notes".into(), notes.into());
                }
                match kind.as_str() {
                    "Hooks" => {
                        entry.insert("name".into(), name.clone().into());
                        let call = function
                            .sig
                            .inputs
                            .iter()
                            .map(|input| match input {
                                syn::FnArg::Typed(input) => Ok(serde_json::json!([
                                    input.pat.to_token_stream().to_string(),
                                    input.ty.to_token_stream().to_string()
                                ])),
                                syn::FnArg::Receiver(_) => {
                                    Err(invalid(format!("{name}: a declaration has no receiver")))
                                }
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        if !call.is_empty() {
                            entry.insert("call".into(), call.into());
                        }
                        if let Some(doc) = doc(&function.attrs) {
                            entry.insert("doc".into(), doc.into());
                        }
                        // The contract's arguments are the entry's `mod_api` object.
                        let mut api = serde_json::Map::new();
                        for key in [
                            "contract",
                            "out",
                            "scope",
                            "key",
                            "answered_by",
                            "owner",
                            "collect_validator",
                            "isolate",
                        ] {
                            if let Some(value) = entry.remove(key) {
                                api.insert(key.into(), value);
                            }
                        }
                        entry.insert("mod_api".into(), api.into());
                        self.hooks.push(entry.into());
                    }
                    _ => {
                        entry.remove("target");
                        if let Some(trait_name) = entry.remove("name") {
                            entry.insert("trait".into(), trait_name);
                        }
                        self.draw_contexts.push(entry.into());
                    }
                }
            }
        }
        Ok(())
    }
}

impl Catalog {
    /// The catalog of the checkout at `root`: every file of the declarations directory, `base.rs` first and
    /// then by name, so the order never depends on a list a PR edits.
    /// The retired JSON catalog is refused wherever it still exists.
    pub fn load(root: &Path) -> Result<Self, CatalogError> {
        let mut catalog = Self::load_declarations(root)?;
        crate::hook_targets::resolve(root, &mut catalog).map_err(|error| {
            CatalogError::Declaration {
                path: "native hook definitions".into(),
                error,
            }
        })?;
        Ok(catalog)
    }

    pub(crate) fn load_declarations(root: &Path) -> Result<Self, CatalogError> {
        let read_error = |path: &Path, e: std::io::Error| CatalogError::Read {
            path: path.display().to_string(),
            error: e.to_string(),
        };
        for retired in RETIRED {
            let path = root.join(retired);
            if path.is_file() {
                return Err(CatalogError::Retired((*retired).to_string()));
            }
            if path.is_dir() {
                let mut entries = fs::read_dir(&path).map_err(|e| read_error(&path, e))?;
                if let Some(entry) = entries.next() {
                    let entry = entry.map_err(|e| read_error(&path, e))?;
                    return Err(CatalogError::Retired(format!(
                        "{retired}/{}",
                        entry.file_name().to_string_lossy()
                    )));
                }
            }
        }
        let dir = root.join(DECLARATIONS);
        let modules = Self::modules(&dir)?;
        let files = modules
            .iter()
            .map(|module| {
                let path = dir.join(format!("{module}.rs"));
                let text = fs::read_to_string(&path).map_err(|e| read_error(&path, e))?;
                Ok((path.display().to_string(), text))
            })
            .collect::<Result<Vec<_>, CatalogError>>()?;
        Self::parse(&files)
    }

    pub(crate) fn modules(dir: &Path) -> Result<Vec<String>, CatalogError> {
        let read_error = |path: &Path, e: std::io::Error| CatalogError::Read {
            path: path.display().to_string(),
            error: e.to_string(),
        };
        let mut modules = fs::read_dir(dir)
            .map_err(|e| read_error(dir, e))?
            .map(|entry| entry.map_err(|e| read_error(dir, e)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".rs")
                    .filter(|module| *module != "mod")
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        // Split declarations retain their original HookId positions in per-item metadata.
        // Unnumbered additions are discovered alphabetically, without a shared registry.
        let mut order = std::collections::HashMap::new();
        for module in &modules {
            let path = dir.join(format!("{module}.rs"));
            let source = fs::read_to_string(&path).map_err(|e| read_error(&path, e))?;
            if let Some(value) = source
                .lines()
                .find_map(|line| line.strip_prefix("// catalog-order: "))
            {
                let ordinal = value
                    .parse::<usize>()
                    .map_err(|e| CatalogError::Declaration {
                        path: path.display().to_string(),
                        error: format!("invalid catalog-order: {e}"),
                    })?;
                if order.values().any(|previous| *previous == ordinal) {
                    return Err(CatalogError::Declaration {
                        path: path.display().to_string(),
                        error: format!("duplicate catalog-order: {ordinal}"),
                    });
                }
                order.insert(module.clone(), ordinal);
            }
        }
        modules.sort_by(|a, b| {
            (a != "base", order.get(a).copied().unwrap_or(usize::MAX), a).cmp(&(
                b != "base",
                order.get(b).copied().unwrap_or(usize::MAX),
                b,
            ))
        });
        for module in &modules {
            if syn::parse_str::<syn::Ident>(module).is_err() {
                return Err(CatalogError::InvalidModule(module.clone()));
            }
        }
        Ok(modules)
    }

    /// The catalog of declaration files given as (path, source), in the order given.
    pub fn parse(files: &[(String, String)]) -> Result<Self, CatalogError> {
        let mut declared = Declared::default();
        for (path, text) in files {
            declared.read(path, text)?;
        }
        let mut seen = HashSet::new();
        let hooks = declared
            .hooks
            .into_iter()
            .map(|raw| {
                let hook = Hook::parse(raw)?;
                if seen.insert(hook.name.clone()) {
                    Ok(hook)
                } else {
                    Err(CatalogError::DuplicateHook(hook.name))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let draw_contexts = declared
            .draw_contexts
            .into_iter()
            .map(|raw| {
                serde_json::from_value(raw).map_err(|e| CatalogError::Declaration {
                    path: "draw context".into(),
                    error: e.to_string(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            hooks,
            draw_contexts,
            general_notes: (!declared.general_notes.is_empty())
                .then(|| declared.general_notes.join("\n\n")),
        })
    }
}

#[cfg(test)]
impl Hook {
    /// A global listener with no counterparts, for tests of what reads a hook.
    pub(crate) fn sample(target: &str, side: Side) -> Self {
        Self {
            name: "test".into(),
            doc: "Test.".into(),
            notes: None,
            side,
            target: target.into(),
            contract: Contract::Listen,
            out: None,
            answered_by_mod: false,
            owner: None,
            collect_validator: None,
            isolate: false,
            scope: None,
            host: None,
            context: None,
            counterparts: Counterparts::default(),
            version: 1,
            cancellable: None,
            call: Vec::new(),
            prepared: None,
            stack_count: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hook(name: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut hook = json!({
            "name": name, "doc": "Doc.", "side": "server",
            "target": "horizon_client::game::native_hooks::Game<'_>::native_x",
            "mod_api": {"contract": "listen"}
        });
        hook.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        hook
    }

    #[test]
    fn a_helpers_trait_is_refused_for_the_host_traits() {
        let source = r#"pub trait Helpers { #[helper(side = "server")] fn example(); }"#;
        assert!(
            super::Catalog::parse(&[("test.rs".into(), source.into())])
                .unwrap_err()
                .to_string()
                .contains("host/<area>.rs")
        );
    }

    #[test]
    fn the_shipped_catalog_loads_with_typed_contracts() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = Catalog::load(&root).unwrap();
        let find = |name: &str| catalog.hooks.iter().find(|h| h.name == name).unwrap();
        let scoped = find("pre_remove_side_effects");
        assert_eq!(scoped.scope.as_ref().unwrap().kind, PartKind::BlockEntity);
        assert_eq!(find("recipes_received").contract, Contract::Listen);
        // `can_eat` changes Player.canEat's answer, so tools route an answer onto it.
        let can_eat = find("can_eat");
        assert_eq!(can_eat.contract, Contract::Replace);
        assert_eq!(can_eat.out.as_deref(), Some("can_eat"));
        assert!(
            catalog
                .hooks
                .iter()
                .any(|h| h.contract == Contract::Replace)
        );
        assert!(!catalog.draw_contexts.is_empty());
    }

    #[test]
    fn prepared_is_rejected_on_other_hooks_at_catalog_boundary() {
        let mut declaration = serde_json::json!({"name":"other_hook","doc":"Doc.","side":"server","target":"unused", "prepared":"stack", "mod_api":{"contract":"listen"}});
        let error = Hook::parse(declaration.clone()).unwrap_err().to_string();
        assert_eq!(
            error,
            "hook other_hook: prepared is supported only as prepared = \"stack\" on can_equip"
        );
        declaration["name"] = "can_equip".into();
        assert_eq!(
            Hook::parse(declaration.clone())
                .unwrap()
                .prepared
                .as_deref(),
            Some("stack")
        );
        declaration["prepared"] = "other".into();
        assert!(
            Hook::parse(declaration)
                .unwrap_err()
                .to_string()
                .contains("only as prepared")
        );
    }

    fn checkout(files: &[(&str, &str)]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "catalog-load-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join(DECLARATIONS);
        fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        root
    }

    const HOOK: &str = "pub trait Hooks { /// Doc.\n #[hook(side = \"server\", contract = \"listen\")] fn one(); }";

    #[test]
    fn the_retired_json_catalog_is_refused_wherever_it_is() {
        let base = format!("{DECLARATIONS}/base.rs");
        let index = format!("{DECLARATIONS}/mod.rs");
        for retired in ["native-hooks.json", "tools/native-hook-fragments/late.json"] {
            let root = checkout(&[(&base, HOOK), (&index, "mod base;"), (retired, "{}")]);
            let error = Catalog::load(&root).unwrap_err().to_string();
            assert!(error.starts_with(retired), "{error}");
            assert!(error.contains("declared in Rust now"), "{error}");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn the_order_comes_from_the_directory_without_a_shared_module_list() {
        let second = "pub trait Hooks { /// Doc.\n #[hook(side = \"server\", contract = \"listen\")] fn two(); }";
        let base = format!("{DECLARATIONS}/base.rs");
        let alpha = format!("{DECLARATIONS}/alpha.rs");
        let index = format!("{DECLARATIONS}/mod.rs");
        let root = checkout(&[
            (&alpha, second),
            (&base, HOOK),
            (&index, "mod base;\nmod alpha;"),
        ]);
        let catalog = Catalog::load_declarations(&root).unwrap();
        let names = catalog
            .hooks
            .iter()
            .map(|h| h.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["one", "two"], "base.rs first, then by name");
        fs::remove_file(root.join(&index)).unwrap();
        assert_eq!(Catalog::load_declarations(&root).unwrap().hooks.len(), 2);
        fs::write(root.join(DECLARATIONS).join("not-valid.rs"), HOOK).unwrap();
        assert!(
            Catalog::load_declarations(&root)
                .unwrap_err()
                .to_string()
                .contains("invalid hook declaration filename")
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn file(name: &str, source: &str) -> (String, String) {
        (name.to_owned(), source.to_owned())
    }

    #[test]
    fn declarations_become_entries_with_the_defaults_a_declaration_leaves_out() {
        let catalog = Catalog::parse(&[file(
            "a.rs",
            r#"
            pub trait Hooks {
                /// First line.
                ///
                ///  Indented.
                #[hook(side = "both", contract = "replace", out = "o", answered_by = "mod",
                       neoforge = ["One", "Two"], cancellable = true, version = 2)]
                fn thing();
                /// Scoped.
                #[hook(side = "server", contract = "listen", scope = "item", key = "stack",
                       target = "horizon_world::loot::native_scoped")]
                fn scoped();
            }
            pub trait DrawContexts {
                /// Draws.
                #[draw_context(surface = "crate::s::S", name = "Draw")]
                fn draw();
            }
            "#,
        )])
        .unwrap();
        let thing = &catalog.hooks[0];
        assert_eq!(thing.doc, "First line.\n\n Indented.");
        assert_eq!(
            thing.target,
            "horizon_client::game::native_hooks::a::Game<'_>::native_thing"
        );
        assert!(thing.answered_by_mod && thing.cancellable == Some(true));
        assert_eq!((thing.version, thing.counterparts.neoforge.len()), (2, 2));
        assert_eq!(
            catalog.hooks[1].target,
            "horizon_world::loot::native_scoped"
        );
        assert_eq!(
            catalog.hooks[1].scope.as_ref().unwrap().kind,
            PartKind::Item
        );
        assert_eq!(catalog.draw_contexts[0].trait_name, "Draw");
    }

    #[test]
    fn a_declaration_repeating_a_hook_is_refused_even_across_files() {
        let declared = |name: &str| {
            format!(
                "pub trait Hooks {{ /// Doc.\n #[hook(side = \"server\", contract = \"listen\")] fn {name}(); }}"
            )
        };
        assert_eq!(
            Catalog::parse(&[
                file("a.rs", &declared("same")),
                file("b.rs", &declared("same"))
            ])
            .unwrap_err(),
            CatalogError::DuplicateHook("same".into())
        );
        assert_eq!(
            Catalog::parse(&[
                file("a.rs", &declared("one")),
                file("b.rs", &declared("two"))
            ])
            .unwrap()
            .hooks
            .len(),
            2
        );
    }

    #[test]
    fn a_declaration_needs_its_attribute_and_its_doc() {
        let parse = |source: &str| {
            Catalog::parse(&[file("a.rs", source)])
                .unwrap_err()
                .to_string()
        };
        assert!(
            parse("pub trait Hooks { fn bare(); }")
                .contains("bare: needs a #[hook(...)] attribute")
        );
        assert_eq!(
            parse(
                "pub trait Hooks { #[hook(side = \"server\", contract = \"listen\")] fn bare(); }"
            ),
            "hook bare: doc missing: add `///` lines, the rustdoc of its trait method"
        );
        assert!(
            parse("pub trait Hooks { /// Doc.\n #[hook(side = \"server\", contract = \"listen\", unknown = 1)] fn bare(); }")
                .contains("unknown field `unknown`")
        );
    }

    #[test]
    fn a_hook_without_a_doc_is_refused() {
        for doc in [json!(null), json!("  ")] {
            assert_eq!(
                Hook::parse(hook("bare", json!({"doc": doc})))
                    .unwrap_err()
                    .to_string(),
                "hook bare: doc missing: add `///` lines, the rustdoc of its trait method"
            );
        }
    }

    #[test]
    fn counterparts_accept_strings_lists_and_absence() {
        let one = Hook::parse(hook(
            "one",
            json!({"fabric": "Event.ONE", "neoforge": "OneEvent"}),
        ))
        .unwrap();
        assert_eq!(one.counterparts.fabric, ["Event.ONE"]);
        assert_eq!(one.counterparts.neoforge, ["OneEvent"]);
        let two =
            Hook::parse(hook("two", json!({"fabric": ["Event.TWO", "Event.THREE"]}))).unwrap();
        assert_eq!(two.counterparts.fabric.len(), 2);
        assert_eq!(
            Hook::parse(hook("none", json!({}))).unwrap().counterparts,
            Counterparts::default()
        );
        for key in ["fabric", "neoforge", "paper"] {
            assert_eq!(
                Hook::parse(hook("bad", json!({key: ["ok", 4]})))
                    .unwrap_err()
                    .to_string(),
                format!("hook bad: {key} must be a string or list of strings")
            );
        }
    }

    #[test]
    fn a_hook_needs_a_side_a_target_and_a_contract() {
        for side in [json!(null), json!("invalid")] {
            assert!(Hook::parse(hook("bad", json!({"side": side}))).is_err());
        }
        for field in ["side", "target", "mod_api"] {
            let mut value = hook("bad", json!({}));
            value.as_object_mut().unwrap().remove(field);
            assert!(
                Hook::parse(value)
                    .unwrap_err()
                    .to_string()
                    .contains(&format!("missing field `{field}`"))
            );
        }
        assert_eq!(
            Hook::parse(json!({})).unwrap_err(),
            CatalogError::NamelessHook
        );
    }

    #[test]
    fn contracts_and_outputs_go_together() {
        let parse = |contract: &str, out: Option<&str>| {
            Hook::parse(hook(
                "c",
                json!({"mod_api": {"contract": contract, "out": out}}),
            ))
        };
        assert_eq!(parse("listen", None).unwrap().contract, Contract::Listen);
        for contract in ["cancel", "collect", "replace"] {
            assert_eq!(
                parse(contract, Some("o")).unwrap().out.as_deref(),
                Some("o")
            );
            assert_eq!(
                parse(contract, None).unwrap_err(),
                CatalogError::InvalidContract("c".into())
            );
        }
        assert_eq!(
            parse("listen", Some("o")).unwrap_err(),
            CatalogError::InvalidContract("c".into())
        );
        assert!(parse("unknown", None).is_err());
    }

    #[test]
    fn a_scope_and_its_key_go_together() {
        let scoped = |api: serde_json::Value| Hook::parse(hook("s", json!({"mod_api": api})));
        let hook_scope = scoped(json!({"contract": "listen", "scope": "item", "key": "stack"}))
            .unwrap()
            .scope
            .unwrap();
        assert_eq!(
            (hook_scope.kind, hook_scope.key.as_str()),
            (PartKind::Item, "stack")
        );
        for api in [
            json!({"contract": "listen", "scope": "item"}),
            json!({"contract": "listen", "key": "stack"}),
        ] {
            assert_eq!(
                scoped(api).unwrap_err(),
                CatalogError::ScopeNeedsKey("s".into())
            );
        }
        assert!(scoped(json!({"contract": "listen", "scope": "nothing", "key": "k"})).is_err());
    }

    #[test]
    fn owner_needs_a_global_listener() {
        let owned = |api: serde_json::Value| Hook::parse(hook("o", json!({"mod_api": api})));
        assert_eq!(
            owned(json!({"contract": "listen", "owner": "mod_id"}))
                .unwrap()
                .owner
                .as_deref(),
            Some("mod_id")
        );
        for api in [
            json!({"contract": "cancel", "out": "c", "owner": "mod_id"}),
            json!({"contract": "listen", "owner": "mod_id", "scope": "item", "key": "k"}),
        ] {
            assert_eq!(
                owned(api).unwrap_err(),
                CatalogError::OwnerNeedsGlobalListener("o".into())
            );
        }
    }

    #[test]
    fn answered_by_needs_a_global_replace_or_collect_hook() {
        let answered = |api: serde_json::Value| Hook::parse(hook("a", json!({"mod_api": api})));
        for contract in ["replace", "collect"] {
            assert!(
                answered(json!({"contract": contract, "out": "o", "answered_by": "mod"}))
                    .unwrap()
                    .answered_by_mod
            );
        }
        for api in [
            json!({"contract": "cancel", "out": "o", "answered_by": "mod"}),
            json!({"contract": "replace", "out": "o", "answered_by": "mod", "scope": "item", "key": "k"}),
            json!({"contract": "collect", "out": "o", "answered_by": "mod", "scope": "item", "key": "k"}),
        ] {
            assert_eq!(
                answered(api).unwrap_err(),
                CatalogError::AnsweredNeedsGlobalOutput("a".into())
            );
        }
    }
    #[test]
    fn split_file_order_is_stable_and_validated() {
        let dir = std::env::temp_dir().join(format!("hook-order-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("z.rs"), "// catalog-order: 1\n").unwrap();
        fs::write(dir.join("a.rs"), "// catalog-order: 2\n").unwrap();
        fs::write(dir.join("b.rs"), "").unwrap();
        assert_eq!(Catalog::modules(&dir).unwrap(), ["z", "a", "b"]);
        fs::write(dir.join("b.rs"), "// catalog-order: 2\n").unwrap();
        assert!(
            Catalog::modules(&dir)
                .unwrap_err()
                .to_string()
                .contains("duplicate catalog-order")
        );
        fs::write(dir.join("b.rs"), "// catalog-order: bad\n").unwrap();
        assert!(
            Catalog::modules(&dir)
                .unwrap_err()
                .to_string()
                .contains("invalid catalog-order")
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod stack_count_tests {
    use super::*;

    #[test]
    fn stack_count_requires_a_stack_collect_output_and_known_policy() {
        let parse = |declaration: &str| Catalog::parse(&[("test.rs".into(), declaration.into())]);
        assert!(
            parse(
                r#"trait Hooks { /// Doc.
            #[hook(side="server", contract="listen", stack_count="trim")]
            fn notice(); }"#
            )
            .is_err()
        );
        assert!(
            parse(
                r#"trait Hooks { /// Doc.
            #[hook(side="server", contract="collect", out="values", stack_count="trim")]
            fn numbers(values: &mut Vec<u32>); }"#
            )
            .is_err()
        );
        assert!(
            parse(
                r#"trait Hooks { /// Doc.
            #[hook(side="server", contract="collect", out="drops", stack_count="unknown")]
            fn fill(drops: &mut Vec<ItemStack>); }"#
            )
            .is_err()
        );
        let catalog = parse(
            r#"trait Hooks { /// Doc.
            #[hook(side="server", contract="collect", out="drops", stack_count="trim")]
            fn fill(drops: &mut Vec<ItemStack>); }"#,
        )
        .unwrap();
        assert_eq!(catalog.hooks[0].stack_count, Some(StackCount::Trim));
    }
}
