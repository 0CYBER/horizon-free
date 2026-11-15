//! What changed between two surfaces, and whether it breaks mods.
//!
//! [`diff`] compares the records of two documents by id. A record only in the new one is
//! `added`, only in the old one `removed`, in both with another hash, or another value of a
//! field the diff compares outside the hash, `changed`. A rename shows as a removal and an
//! addition: renames are never guessed.
//!
//! # Classes
//!
//! - **Breaking**: it can stop any mod that uses the item from compiling or meaning the same.
//!   A removed record; a changed contract (a hook's side or kind, a retyped field or argument, a
//!   signature, a method no longer on wasm); a variant added to an exhaustive enum or a required
//!   item added to a trait mods implement (the container's own hash covers those sets); an impl
//!   a type no longer has; a supertrait a trait mods call no longer has, or gains when the new
//!   one is not known to keep `dyn Trait` valid (`Clone`, `Sized`, a trait with generic methods).
//! - **Conditional**: it breaks only some mods, which the fit test finds from each mod's source.
//!   A field added to a struct mods can build, or an argument added to a hook (only a mod that
//!   builds the value with a full literal or matches it with an exhaustive pattern, without
//!   `..`), or a struct's public field set changing in a field its records do not list (its
//!   `fields_hash`); a struct that is no longer
//!   constructible (only a mod that builds it with a literal); a public path that goes (only a
//!   mod that names the item by it); a hook's `version` (only a mod whose manifest names it) or a
//!   behaviour's (only a mod that relies on it); a
//!   type's wire form or wire layout (`shape`; wasm mods only); a method of a trait mods only call
//!   that moved unchanged to a supertrait it still has (only a call written by trait path).
//! - **Additive**: a new record (a behaviour, a field of a struct mods cannot build, a variant of a
//!   `#[non_exhaustive]` enum, a defaulted method), and changes that only widen an item: now on
//!   wasm, now crossing the wasm boundary, now constructible, an impl gained, a new public path,
//!   a marker or object-safe API supertrait gained by a trait mods only call (`Send`, `Sync`,
//!   `'static`, `Debug`, `Any`), an item no longer required, an enum no longer
//!   `#[non_exhaustive]`.
//!
//! A container's members are reported with it once: when a type, trait or hook is added or
//! removed, or its hashed member set (`members`, `required`, whether a hook has an output)
//! changed, its members' own changes are folded into its change (listed in `members`, and a
//! changed or removed member's lines repeated under the container with its name) and its class
//! is the worst of all of them.
//!
//! Each change says what changed in words a mod author can act on, from the records' contract
//! fields: `argument `player`: `PlayerRef` → `ServerPlayerRef``, `side: server → both`.
use crate::{Role, Surface, TypeKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Whether a change can break a mod that compiled against the old surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Additive,
    /// Breaks only a mod that builds the value with a full literal, names a hook version, or
    /// is a wasm mod; the fit test judges each mod.
    Conditional,
    Breaking,
}

/// The whole comparison in one word: its worst class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Identical,
    Additive,
    Conditional,
    Breaking,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
}

/// A note's text, attached to the changes it names (`crates/horizon-mod-vocab/changes/*.toml`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The note's file name.
    pub file: String,
    pub why: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrate: Option<String>,
}

/// One record's change, with its folded members.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub id: String,
    pub kind: ChangeKind,
    pub class: Class,
    /// What changed, one line per fact.
    pub what: Vec<String>,
    /// The ids of member records whose own change is reported here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
}

/// The comparison of two surfaces: breaking changes first, then conditional, then additive,
/// each by id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    /// The old document's hash.
    pub old: String,
    /// The new document's hash.
    pub new: String,
    pub verdict: Verdict,
    pub changes: Vec<Change>,
}

/// A record as the diff sees it.
struct Entry {
    hash: String,
    /// The hashed part of its contract, as JSON.
    view: Value,
    /// What the diff compares outside the hash: public `paths`; a type's `impls`, `wire`,
    /// `wire_form`, `shape`, `constructible` and `fields_hash`; a called trait's `bounds`; a
    /// hook's or behaviour's `version`.
    extra: Map<String, Value>,
}

fn json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("contracts serialize")
}

/// The keys of `value` (an object) moved into a map of their own.
fn take(value: &mut Value, keys: &[&str]) -> Map<String, Value> {
    let object = value.as_object_mut().expect("contracts are objects");
    keys.iter()
        .filter_map(|k| object.remove(*k).map(|v| ((*k).to_owned(), v)))
        .collect()
}

/// The public paths a mod can name a record by, compared outside the hash.
fn paths(paths: &[String]) -> Map<String, Value> {
    Map::from_iter([("paths".to_owned(), json(&paths))])
}

fn entries(surface: &Surface) -> BTreeMap<String, Entry> {
    let mut map = BTreeMap::new();
    let mut put = |id: &str, hash: &str, view: Value, extra: Map<String, Value>| {
        map.insert(
            id.to_owned(),
            Entry {
                hash: hash.to_owned(),
                view,
                extra,
            },
        );
    };
    for r in &surface.hooks {
        // A hook's arguments and output are member records; its hash holds only whether it
        // has an output.
        let mut view = json(&r.contract);
        let extra = take(&mut view, &["version"]);
        take(&mut view, &["arguments", "refs", "output"]);
        view["output"] = Value::Bool(r.contract.output.is_some());
        put(&r.id, &r.hash, view, extra);
        for f in &r.fields {
            put(&f.id, &f.hash, json(&f.contract), Map::new());
        }
    }
    for r in &surface.traits {
        let mut view = json(&r.contract);
        let mut extra = paths(&r.paths);
        // A trait mods only call keeps its supertraits out of the hash: one it gains asks
        // nothing of a caller, but one it loses can break a caller that relied on it.
        if r.contract.role == Role::Call {
            take(&mut view, &["bounds", "refs"]);
            extra.insert("bounds".into(), json(&r.contract.bounds));
        }
        put(&r.id, &r.hash, view, extra);
        for m in &r.methods {
            put(&m.id, &m.hash, json(&m.contract), Map::new());
        }
        for a in &r.associated {
            put(&a.id, &a.hash, json(&a.contract), Map::new());
        }
    }
    for r in surface.handles.iter().chain(&surface.functions) {
        put(&r.id, &r.hash, json(&r.contract), paths(&r.paths));
    }
    for r in &surface.macros {
        put(&r.id, &r.hash, json(&r.contract), paths(&r.paths));
    }
    for r in &surface.types {
        let mut view = json(&r.contract);
        let mut extra = take(&mut view, &["wire", "wire_form", "constructible"]);
        if r.contract.kind == TypeKind::Enum {
            // An enum's `constructible` is its `non_exhaustive`, which stays in the view.
            extra.remove("constructible");
        } else {
            // A struct's `non_exhaustive` shows only through `constructible`, as in its hash.
            take(&mut view, &["non_exhaustive"]);
        }
        extra.insert("shape".into(), json(&r.shape));
        extra.insert("impls".into(), json(&r.impls));
        extra.extend(paths(&r.paths));
        if let Some(fields) = &r.fields_hash {
            extra.insert("fields_hash".into(), fields.as_str().into());
        }
        put(&r.id, &r.hash, view, extra);
        for f in &r.fields {
            put(&f.id, &f.hash, json(&f.contract), Map::new());
        }
        for v in &r.variants {
            put(&v.id, &v.hash, json(&v.contract), Map::new());
        }
    }
    for r in &surface.parts {
        put(&r.id, &r.hash, json(&r.contract), Map::new());
    }
    for r in &surface.capabilities {
        put(&r.id, &r.hash, json(&r.contract), Map::new());
    }
    for r in &surface.behaviours {
        // A behaviour's hash holds nothing: its version is all the diff compares.
        let mut view = json(&r.contract);
        let extra = take(&mut view, &["version"]);
        put(&r.id, &r.hash, view, extra);
    }
    map
}

/// The container of a member record: a trait method's or associated item's trait, a field's
/// or variant's type, a hook argument's hook.
fn container(id: &str) -> Option<String> {
    let (kind, rest) = id.split_once(':')?;
    match kind {
        "method" | "assoc" => Some(format!("trait:{}", rest.rsplit_once("::")?.0)),
        "type" | "hook" => Some(format!("{kind}:{}", rest.rsplit_once('.')?.0)),
        _ => None,
    }
}

/// For a method gone from a trait mods only call: the trait and the supertrait it moved to
/// unchanged, when the trait still has that supertrait in `new`, so a method call still compiles
/// and only a call written by trait path (`ServerHost::spawn(host)`) breaks.
fn moved_to(
    id: &str,
    old: &Entry,
    new: &Surface,
    after: &BTreeMap<String, Entry>,
) -> Option<(String, String)> {
    let (path, method) = id.strip_prefix("method:")?.rsplit_once("::")?;
    let trait_of = |bound: &str| {
        // A bound is written as the trait's path, perhaps with generics.
        let bound = bound.split('<').next().unwrap_or(bound).trim();
        new.traits
            .iter()
            .find(|t| t.id.strip_prefix("trait:") == Some(bound))
    };
    let start = new
        .traits
        .iter()
        .find(|t| t.id.strip_prefix("trait:") == Some(path) && t.contract.role == Role::Call)?;
    let mut seen = BTreeSet::new();
    let mut queue: Vec<&crate::Trait> = vec![start];
    while let Some(t) = queue.pop() {
        for bound in &t.contract.bounds {
            let Some(sup) = trait_of(bound) else { continue };
            if !seen.insert(sup.id.as_str()) {
                continue;
            }
            let sup_path = sup.id.strip_prefix("trait:").unwrap_or(&sup.id);
            let moved = after.get(&format!("method:{sup_path}::{method}"));
            if moved.is_some_and(|m| m.view == old.view) {
                return Some((
                    crate::leaf(&start.id).to_owned(),
                    crate::leaf(&sup.id).to_owned(),
                ));
            }
            queue.push(sup);
        }
    }
    None
}

/// Supertraits a trait mods only call can gain without breaking a caller: they keep `dyn Trait`
/// valid and ask nothing of the caller. The marker traits, and every API trait in `surface`
/// whose own record shows it is object safe (methods on `self` with no generics and no `Self`,
/// no associated items) and whose supertraits are free too. Anything the record cannot show is
/// left out.
fn free_bounds(surface: &Surface) -> BTreeSet<String> {
    let mut free: BTreeSet<String> = ["Send", "Sync", "'static", "Debug", "Any"]
        .into_iter()
        .map(String::from)
        .collect();
    let safe_method = |signature: &str| {
        let after_name = signature.strip_prefix("fn ").and_then(|rest| {
            rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map(|i| &rest[i..])
        });
        after_name.is_some_and(|rest| {
            (rest.starts_with("(&self") || rest.starts_with("(&mut self"))
                && !rest.contains("Self")
                && !rest.contains("impl ")
        })
    };
    loop {
        let before = free.len();
        for t in &surface.traits {
            let path = t.id.strip_prefix("trait:").unwrap_or(&t.id);
            if free.contains(path)
                || !t.contract.generics.is_empty()
                || !t.associated.is_empty()
                || !t.methods.iter().all(|m| safe_method(&m.contract.signature))
                || !t.contract.bounds.iter().all(|b| free.contains(b.as_str()))
            {
                continue;
            }
            free.insert(path.to_owned());
        }
        if free.len() == before {
            return free;
        }
    }
}

/// The containers in both surfaces whose new members a mod's full literal must name: every
/// hook (its event) and every struct mods can build.
fn literal_owners(old: &Surface, new: &Surface) -> BTreeSet<String> {
    let builds = |surface: &Surface| -> BTreeSet<String> {
        let structs = surface.types.iter().filter(|t| {
            t.contract.constructible
                && matches!(t.contract.kind, TypeKind::Struct | TypeKind::TupleStruct)
        });
        surface
            .hooks
            .iter()
            .map(|h| h.id.clone())
            .chain(structs.map(|t| t.id.clone()))
            .collect()
    };
    builds(old).intersection(&builds(new)).cloned().collect()
}

/// Compares two surfaces.
pub fn diff(old: &Surface, new: &Surface) -> Diff {
    let before = entries(old);
    let after = entries(new);
    let owners = literal_owners(old, new);
    let free = free_bounds(new);
    let ids: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let mut changes: BTreeMap<String, Change> = BTreeMap::new();
    for id in ids {
        let (kind, lines) = match (before.get(id), after.get(id)) {
            (Some(a), None) => {
                let line = match moved_to(id, a, new, &after) {
                    Some((from, to)) => (
                        format!(
                            "moved to the supertrait `{to}`: breaks only calls written by trait path (`{from}::{}(host)`)",
                            crate::leaf(id)
                        ),
                        Class::Conditional,
                    ),
                    None => ("removed".into(), Class::Breaking),
                };
                (ChangeKind::Removed, vec![line])
            }
            (None, Some(b)) => {
                let line = match container(id).filter(|owner| owners.contains(owner)) {
                    Some(owner) => (literal_added(id, &owner, &b.view), Class::Conditional),
                    None => ("added".into(), Class::Additive),
                };
                (ChangeKind::Added, vec![line])
            }
            (Some(a), Some(b)) => {
                let mut lines = if a.hash == b.hash {
                    Vec::new()
                } else {
                    describe(id, &a.view, &b.view)
                };
                lines.extend(extras(id, &a.extra, &b.extra, &free));
                if a.hash != b.hash && lines.is_empty() {
                    lines.push((
                        "its contract changed in a field this tool does not know".into(),
                        Class::Breaking,
                    ));
                }
                if lines.is_empty() {
                    continue;
                }
                (ChangeKind::Changed, lines)
            }
            (None, None) => continue,
        };
        let class = lines
            .iter()
            .map(|(_, c)| *c)
            .max()
            .expect("one line at least");
        changes.insert(
            id.clone(),
            Change {
                id: id.clone(),
                kind,
                class,
                what: lines.into_iter().map(|(line, _)| line).collect(),
                members: Vec::new(),
                notes: Vec::new(),
            },
        );
    }
    // A constructible struct whose public field set changed with no field record to show it
    // (a field mods can't see in the docs was added or removed): a full literal stops building.
    let fields_moved: BTreeSet<String> = changes
        .keys()
        .filter(|id| id.starts_with("type:"))
        .filter_map(|id| container(id))
        .collect();
    for (id, a) in &before {
        let Some(b) = after.get(id) else { continue };
        let open = |e: &Entry| e.extra.get("constructible") == Some(&Value::Bool(true));
        if a.extra.get("fields_hash") == b.extra.get("fields_hash")
            || !open(a)
            || !open(b)
            || fields_moved.contains(id)
        {
            continue;
        }
        let line = format!(
            "its public field set changed in a field its records do not list: breaks only a mod that builds `{}` with a full literal or matches it with an exhaustive pattern (no `..`)",
            crate::leaf(id)
        );
        let change = changes.entry(id.clone()).or_insert_with(|| Change {
            id: id.clone(),
            kind: ChangeKind::Changed,
            class: Class::Conditional,
            what: Vec::new(),
            members: Vec::new(),
            notes: Vec::new(),
        });
        change.class = change.class.max(Class::Conditional);
        change.what.push(line);
    }
    // Fold members into a container that was added or removed, or whose member set changed.
    let folds: BTreeSet<String> = changes
        .values()
        .filter(|c| {
            c.kind != ChangeKind::Changed || {
                let (a, b) = (&before[&c.id].view, &after[&c.id].view);
                ["members", "required", "output"].iter().any(|k| {
                    a.get(k).filter(|v| !v.is_string()) != b.get(k).filter(|v| !v.is_string())
                })
            }
        })
        .map(|c| c.id.clone())
        .collect();
    let members: Vec<(String, String)> = changes
        .keys()
        .filter_map(|id| Some((id.clone(), container(id)?)))
        .filter(|(_, owner)| folds.contains(owner))
        .collect();
    for (id, owner) in members {
        let member = changes.remove(&id).expect("listed above");
        let owner = changes.get_mut(&owner).expect("folds are changes");
        owner.class = owner.class.max(member.class);
        // A member added with its container, or to a changed one, is told by the container's
        // own lines; a member changed or removed keeps its words unless they already name it.
        let name = crate::leaf(&id);
        let told = owner
            .what
            .iter()
            .any(|line| line.contains(&format!("`{name}`")));
        if owner.kind == ChangeKind::Changed && member.kind != ChangeKind::Added && !told {
            owner
                .what
                .extend(member.what.iter().map(|line| format!("`{name}`: {line}")));
        }
        owner.members.push(id);
    }
    let mut changes: Vec<Change> = changes.into_values().collect();
    changes.sort_by(|a, b| b.class.cmp(&a.class).then_with(|| a.id.cmp(&b.id)));
    let verdict = match changes.iter().map(|c| c.class).max() {
        None => Verdict::Identical,
        Some(Class::Additive) => Verdict::Additive,
        Some(Class::Conditional) => Verdict::Conditional,
        Some(Class::Breaking) => Verdict::Breaking,
    };
    Diff {
        old: old.hash.clone(),
        new: new.hash.clone(),
        verdict,
        changes,
    }
}

/// The line for a member added to a container a mod may build with a literal.
fn literal_added(id: &str, owner: &str, view: &Value) -> String {
    let member = format!("{}: {}", crate::leaf(id), show(&view["type"]));
    let name = crate::leaf(owner);
    if owner.starts_with("hook:") {
        format!(
            "argument `{member}` added to `{name}`: breaks only a mod that builds this hook's event with a full literal or matches it with an exhaustive pattern (no `..`)"
        )
    } else {
        format!(
            "field `{member}` added to `{name}`: breaks only a mod that builds `{name}` with a full literal or matches it with an exhaustive pattern (no `..`)"
        )
    }
}

/// What changed outside the hash: impls, public paths, whether a struct is constructible, a
/// hook's version, the wire form.
fn extras(
    id: &str,
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    free: &BTreeSet<String>,
) -> Vec<(String, Class)> {
    let name = crate::leaf(id);
    let get = |map: &Map<String, Value>, key: &str| map.get(key).cloned().unwrap_or(Value::Null);
    let mut lines = Vec::new();
    let strings = |map: &Map<String, Value>, key: &str| -> Vec<String> {
        match map.get(key) {
            Some(Value::Array(items)) => items.iter().map(show).collect(),
            _ => Vec::new(),
        }
    };
    let (x, y) = (strings(old, "impls"), strings(new, "impls"));
    for gained in y.iter().filter(|t| !x.contains(t)) {
        lines.push((format!("now implements `{gained}`"), Class::Additive));
    }
    for lost in x.iter().filter(|t| !y.contains(t)) {
        lines.push((
            format!("the type `{name}` no longer implements `{lost}`"),
            Class::Breaking,
        ));
    }
    let (x, y) = (strings(old, "paths"), strings(new, "paths"));
    for gained in y.iter().filter(|p| !x.contains(p)) {
        lines.push((
            format!("now also exported as `mod_api::{gained}`"),
            Class::Additive,
        ));
    }
    for lost in x.iter().filter(|p| !y.contains(p)) {
        lines.push((
            format!(
                "`mod_api::{lost}` is no longer exported under that path: affects only a mod that names it by that path"
            ),
            Class::Conditional,
        ));
    }
    if let (Some(Value::Array(_)), Some(Value::Array(_))) = (old.get("bounds"), new.get("bounds")) {
        let (x, y) = (strings(old, "bounds"), strings(new, "bounds"));
        for gained in y.iter().filter(|b| !x.contains(b)) {
            lines.push(if free.contains(gained.as_str()) {
                (format!("now also requires `{gained}`"), Class::Additive)
            } else {
                (
                    format!("now also requires `{gained}`: `dyn {name}` stops compiling"),
                    Class::Breaking,
                )
            });
        }
        for lost in x.iter().filter(|b| !y.contains(b)) {
            lines.push((
                format!(
                    "no longer requires `{lost}`: a mod that relies on `{name}: {lost}` or calls a method of `{lost}` through it no longer compiles"
                ),
                Class::Breaking,
            ));
        }
    }
    match (get(old, "constructible"), get(new, "constructible")) {
        (Value::Bool(false), Value::Bool(true)) => {
            lines.push(("now constructible with a literal".into(), Class::Additive))
        }
        (Value::Bool(true), Value::Bool(false)) => lines.push((
            "no longer constructible: breaks only a mod that builds it with a literal".into(),
            Class::Conditional,
        )),
        _ => {}
    }
    let (x, y) = (get(old, "version"), get(new, "version"));
    if x != y {
        let affects = if id.starts_with("behaviour:") {
            "a mod that relies on this behaviour"
        } else {
            "a mod that names this hook's version in its manifest"
        };
        lines.push((
            format!(
                "version {} → {}: affects only {affects}",
                show(&x),
                show(&y)
            ),
            Class::Conditional,
        ));
    }
    let (x, y) = (get(old, "shape"), get(new, "shape"));
    if x != y {
        lines.push((
            format!(
                "wire layout: {} → {}: affects wasm mods only",
                quoted("shape", &x),
                quoted("shape", &y)
            ),
            Class::Conditional,
        ));
    }
    let wire = |map: &Map<String, Value>| get(map, "wire") == Value::Bool(true);
    match (wire(old), wire(new)) {
        (false, true) => lines.push(("now crosses the wasm boundary".into(), Class::Additive)),
        (true, false) => lines.push((
            "no longer crosses the wasm boundary: affects wasm mods only".into(),
            Class::Conditional,
        )),
        _ => {
            let (x, y) = (get(old, "wire_form"), get(new, "wire_form"));
            if x != y {
                lines.push((
                    format!(
                        "wire form: {} → {}: affects wasm mods only",
                        quoted("wire_form", &x),
                        quoted("wire_form", &y)
                    ),
                    Class::Conditional,
                ));
            }
        }
    }
    lines
}

/// Keys whose values are words, shown without code quotes.
const PLAIN: &[&str] = &[
    "side", "contract", "role", "kind", "context", "host", "version",
];

fn label(key: &str) -> String {
    match key {
        "contract" => "kind".into(),
        "answered_by_mod" => "output names the mod that answered".into(),
        "impl_generics" => "impl generics".into(),
        "wire_form" => "wire form".into(),
        "trait" => "Part trait".into(),
        _ => key.replace('_', " "),
    }
}

/// A value as text: `none` for nothing, `name: type` for a named value.
fn show(value: &Value) -> String {
    match value {
        Value::Null => "none".into(),
        Value::String(s) if s.is_empty() => "none".into(),
        Value::String(s) => s.clone(),
        Value::Object(o) => match (o.get("name"), o.get("type"), o.get("key")) {
            (Some(Value::String(n)), Some(Value::String(t)), _) => format!("{n}: {t}"),
            (_, _, Some(Value::String(k))) => format!(
                "{} `{k}`",
                o.get("kind").and_then(Value::as_str).unwrap_or("")
            ),
            _ => Value::Object(o.clone()).to_string(),
        },
        other => other.to_string(),
    }
}

fn quoted(key: &str, value: &Value) -> String {
    let text = show(value);
    if PLAIN.contains(&key)
        || matches!(value, Value::Null | Value::Bool(_) | Value::Number(_))
        || text == "none"
    {
        text
    } else {
        format!("`{text}`")
    }
}

/// A missing key means the type's empty value: `false`, `[]`, or nothing.
fn get(object: &Map<String, Value>, other: &Map<String, Value>, key: &str) -> Value {
    match (object.get(key), other.get(key)) {
        (Some(v), _) => v.clone(),
        (None, Some(Value::Bool(_))) => Value::Bool(false),
        (None, Some(Value::Array(_))) => Value::Array(Vec::new()),
        _ => Value::Null,
    }
}

/// The leading identifier of a member (`name: Type`, `Name(A)`, `Name { a: A }`).
fn member_name(text: &str) -> &str {
    let end = text
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    &text[..end]
}

/// What changed in a record's contract, one line per fact with its class.
fn describe(id: &str, old: &Value, new: &Value) -> Vec<(String, Class)> {
    use Class::{Additive, Breaking};
    let (Value::Object(a), Value::Object(b)) = (old, new) else {
        return vec![(format!("{} → {}", show(old), show(new)), Breaking)];
    };
    let name = crate::leaf(id);
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    let mut lines = Vec::new();
    let mut refs_changed = false;
    // An enum lists its variants only while it is exhaustive.
    let listing = get(a, b, "non_exhaustive") != get(b, a, "non_exhaustive");
    for key in keys {
        let (x, y) = (get(a, b, key), get(b, a, key));
        if x == y || (listing && key == "members") {
            continue;
        }
        let key = key.as_str();
        match (key, &x, &y) {
            ("refs", ..) => refs_changed = true,
            // A struct field's or hook argument's type.
            ("type", ..) if container(id).is_some() => {
                let word = if id.starts_with("hook:") {
                    "argument"
                } else {
                    "field"
                };
                lines.push((
                    format!("{word} `{name}`: {} → {}", quoted(key, &x), quoted(key, &y)),
                    Breaking,
                ));
            }
            ("members", Value::Array(x), Value::Array(y)) => {
                // Only an exhaustive enum lists its members in its contract.
                let consequence =
                    format!("an exhaustive `match` on `{name}` in a mod no longer compiles");
                let x: Vec<&str> = x.iter().filter_map(Value::as_str).collect();
                let y: Vec<&str> = y.iter().filter_map(Value::as_str).collect();
                for line in pair(
                    &x,
                    &y,
                    "variant",
                    |s| member_name(s).to_owned(),
                    |s| (*s).to_owned(),
                ) {
                    let line = if line.ends_with(" added") {
                        format!("{line}: {consequence}")
                    } else {
                        line
                    };
                    lines.push((line, Breaking));
                }
            }
            ("required", Value::Array(x), Value::Array(y)) => {
                let names = |v: &Vec<Value>| -> Vec<(String, String)> {
                    v.iter()
                        .map(|r| (show(&r["name"]), show(&r["signature"])))
                        .collect()
                };
                let (x, y) = (names(x), names(y));
                let old: BTreeMap<&str, &str> =
                    x.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
                let new: BTreeMap<&str, &str> =
                    y.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
                for (n, s) in &y {
                    match old.get(n.as_str()) {
                        None => lines.push((
                            format!("required `{s}` added: every implementation of `{name}` must write it"),
                            Breaking,
                        )),
                        Some(o) if o != s => lines.push((
                            format!("required `{n}`: `{o}` → `{s}`"),
                            Breaking,
                        )),
                        _ => {}
                    }
                }
                for (n, _) in &x {
                    if !new.contains_key(n.as_str()) {
                        lines.push((format!("`{n}` is no longer required"), Additive));
                    }
                }
            }
            ("fields", Value::Array(x), Value::Array(y)) => {
                for line in pair(x, y, "field", |v| show(&v["name"]), show) {
                    lines.push((line, Breaking));
                }
            }
            (_, Value::Array(x), Value::Array(y)) => {
                let word = match key {
                    "bounds" => "supertrait",
                    "attributes" => "helper attribute",
                    _ => key,
                };
                let x: Vec<String> = x.iter().map(show).collect();
                let y: Vec<String> = y.iter().map(show).collect();
                let added: Vec<&String> = y.iter().filter(|v| !x.contains(v)).collect();
                let removed: Vec<&String> = x.iter().filter(|v| !y.contains(v)).collect();
                for v in &added {
                    let class = if key == "bounds" { Breaking } else { Additive };
                    lines.push((format!("{word} `{v}` added"), class));
                }
                for v in &removed {
                    lines.push((format!("{word} `{v}` removed"), Breaking));
                }
                if added.is_empty() && removed.is_empty() {
                    lines.push((format!("{word}s reordered"), Breaking));
                }
            }
            ("wasm", _, Value::Bool(true)) => lines.push(("now on wasm".into(), Additive)),
            ("wasm", _, Value::Bool(false)) => lines.push(("no longer on wasm".into(), Breaking)),
            ("required", _, Value::Bool(false)) => {
                lines.push(("now has a default".into(), Additive))
            }
            ("required", _, Value::Bool(true)) => lines.push((
                "no longer has a default: every implementation must write it".into(),
                Breaking,
            )),
            ("non_exhaustive", _, Value::Bool(true)) => {
                lines.push(("now `#[non_exhaustive]`".into(), Breaking))
            }
            ("non_exhaustive", _, Value::Bool(false)) => {
                lines.push(("no longer `#[non_exhaustive]`".into(), Additive))
            }
            ("output", _, Value::Bool(true)) => lines.push(("now has an output".into(), Breaking)),
            ("output", _, Value::Bool(false)) => {
                lines.push(("no longer has an output".into(), Breaking))
            }
            _ => lines.push((
                format!("{}: {} → {}", label(key), quoted(key, &x), quoted(key, &y)),
                Breaking,
            )),
        }
    }
    if refs_changed && lines.is_empty() {
        lines.push(("the API types it names changed".into(), Breaking));
    }
    lines
}

/// Lines for two lists of named things, paired by name: added, removed, changed, reordered.
fn pair<T>(
    old: &[T],
    new: &[T],
    word: &str,
    name: impl Fn(&T) -> String,
    text: impl Fn(&T) -> String,
) -> Vec<String> {
    let strip = |n: &str, t: String| match t.strip_prefix(&format!("{n}: ")) {
        Some(rest) => rest.to_owned(),
        None => t,
    };
    let before: BTreeMap<String, String> = old.iter().map(|v| (name(v), text(v))).collect();
    let after: BTreeMap<String, String> = new.iter().map(|v| (name(v), text(v))).collect();
    let mut lines = Vec::new();
    for v in new {
        let n = name(v);
        let t = text(v);
        match before.get(&n) {
            None => lines.push(format!("{word} `{t}` added")),
            Some(o) if *o != t => lines.push(format!(
                "{word} `{n}`: `{}` → `{}`",
                strip(&n, o.clone()),
                strip(&n, t)
            )),
            _ => {}
        }
    }
    for v in old {
        if !after.contains_key(&name(v)) {
            lines.push(format!("{word} `{}` removed", name(v)));
        }
    }
    let order = |list: &[T], other: &BTreeMap<String, String>| -> Vec<String> {
        list.iter()
            .map(&name)
            .filter(|n| other.contains_key(n))
            .collect()
    };
    if lines.is_empty() && order(old, &after) != order(new, &before) {
        lines.push(format!("{word}s reordered"));
    }
    lines
}

impl Diff {
    /// Attaches a note to the changes it names; returns the ids it names that did not change.
    pub fn attach(&mut self, note: &Note, ids: &[String]) -> Vec<String> {
        let mut stale = Vec::new();
        for id in ids {
            let change = self
                .changes
                .iter_mut()
                .find(|c| c.id == *id || c.members.contains(id));
            match change {
                Some(change) if !change.notes.contains(note) => change.notes.push(note.clone()),
                Some(_) => {}
                None => stale.push(id.clone()),
            }
        }
        stale
    }

    /// The breaking changes no note declares.
    pub fn undeclared(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|c| c.class == Class::Breaking && c.notes.is_empty())
    }

    /// The report as text: the verdict and counts, then breaking, conditional and additive changes.
    pub fn to_text(&self) -> String {
        let count = |class| self.changes.iter().filter(|c| c.class == class).count();
        let (breaking, conditional, additive) = (
            count(Class::Breaking),
            count(Class::Conditional),
            count(Class::Additive),
        );
        let verdict = match self.verdict {
            Verdict::Identical => "identical",
            Verdict::Additive => "additive",
            Verdict::Conditional => "conditional",
            Verdict::Breaking => "breaking",
        };
        let mut text = format!(
            "Surface: {verdict} ({breaking} breaking, {conditional} conditional, {additive} additive; {} breaking without a note)\nOld {}, new {}\n",
            self.undeclared().count(),
            self.old,
            self.new
        );
        for (class, title, n) in [
            (Class::Breaking, "Breaking", breaking),
            (
                Class::Conditional,
                "Conditional: breaks only some mods (the fit test names them; a behaviour's version is for the mods that rely on it)",
                conditional,
            ),
            (Class::Additive, "Additive", additive),
        ] {
            if n == 0 {
                continue;
            }
            text.push_str(&format!("\n{title} ({n})\n"));
            for change in self.changes.iter().filter(|c| c.class == class) {
                let kind = match change.kind {
                    ChangeKind::Added => "added",
                    ChangeKind::Removed => "removed",
                    ChangeKind::Changed => "changed",
                };
                text.push_str(&format!("- {} ({kind})\n", change.id));
                for line in &change.what {
                    if line != kind {
                        text.push_str(&format!("    {line}\n"));
                    }
                }
                if !change.members.is_empty() {
                    text.push_str(&format!("    members: {}\n", change.members.join(", ")));
                }
                for note in &change.notes {
                    text.push_str(&format!("    why ({}): {}\n", note.file, note.why));
                    if let Some(migrate) = &note.migrate {
                        text.push_str(&format!("    migrate: {migrate}\n"));
                    }
                }
            }
        }
        text
    }
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
