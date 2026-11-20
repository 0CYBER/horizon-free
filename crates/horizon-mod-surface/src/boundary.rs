//! What a wasm mod and the game check each other by when the mod loads, as the generator
//! computes it, and the changes to it that refuse mods built before.
//!
//! The game compares each call a mod imports or exports by its key and its hash: the call's
//! signature hash, which writes out every borrowing record it reaches field by field in
//! declaration order, mixed with the run-time `Wire` layout of every owned type it reaches. Every
//! mod is also checked by the core hash: the transport's values and control payloads, the
//! hand-written codecs, the NBT tag ids and limits, the file roots and the prepared ABI. A minimal
//! mod imports nearly every call, so a change to an existing call or to the core refuses every
//! wasm mod built before it.
//!
//! [`Boundary`] holds what the source says of those: each call's signature hash, the owned types
//! it reaches with their [shapes](crate::shape) (hidden ones too), and a fingerprint of the core's
//! source inputs with the owned types its control payloads reach. [`apply`] adds every change to
//! an existing call or to the core to a surface diff, as breaking. A call that is new is never a
//! change here.
use crate::Surface;
use crate::diff::{Change, ChangeKind, Class, Diff, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The schema of a boundary document; another number is refused.
pub const SCHEMA: u32 = 1;

/// The calls of one build and its core fingerprint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Boundary {
    pub schema: u32,
    /// A fingerprint of everything the core hash is made of that the source holds, except the
    /// layouts of `core_owned`.
    pub core: String,
    /// The owned types the core's control payloads reach (`PackageInfo`, which a mod's start
    /// receives).
    pub core_owned: Vec<Owned>,
    pub calls: Vec<Call>,
}

/// One call both sides know.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    /// `Owner::method`, `held-Owner::method` for a method of a handler another mod gave.
    pub key: String,
    /// The generator's signature hash, an export's mixed with its id, as 16 hex digits.
    pub signature: String,
    pub export: bool,
    /// The owned types it reaches through its arguments, answer and borrowing records, sorted by
    /// path.
    pub owned: Vec<Owned>,
}

/// An owned type a call or the core reaches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owned {
    /// As the generated glue names it (`mod_api::PlayerWorldRefusal`).
    pub path: String,
    /// Its wire layout hash; `None` for a type that crosses by a hand-written codec, which the
    /// core fingerprint covers.
    pub shape: Option<String>,
}

impl Boundary {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a boundary serializes") + "\n"
    }

    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        let boundary: Boundary = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if boundary.schema != SCHEMA {
            return Err(format!("boundary schema {}, not {SCHEMA}", boundary.schema));
        }
        Ok(boundary)
    }
}

const REFUSED: &str = "every wasm mod built before it is refused at load";

/// The id a call's change is reported under: the surface record of the trait method it is
/// (`method:Hooks::player_move`, `method:prelude::PlayersServerHost::profile_by_name`), so a
/// note names it as it names any other change; `call:<key>` where the surface has none.
fn call_id(key: &str, surfaces: [&Surface; 2]) -> String {
    let key = key.strip_prefix("held-").unwrap_or(key);
    let exact = format!("method:{key}");
    let suffix = format!("::{key}");
    let mut found = None;
    for surface in surfaces {
        for method in surface.traits.iter().flat_map(|t| &t.methods) {
            if method.id == exact {
                return exact;
            }
            if method.id.ends_with(&suffix) && found.is_none() {
                found = Some(method.id.clone());
            }
        }
    }
    found.unwrap_or_else(|| format!("call:{key}"))
}

/// The id an owned type's change is reported under: its surface record
/// (`mod_api::item::Stack<T>` is `type:item::Stack` or a type exported as `item::Stack`), or
/// `type:<path>` for a hidden one.
fn type_id(path: &str, surfaces: [&Surface; 2]) -> String {
    let path = path.split('<').next().unwrap_or(path).trim();
    let path = path
        .strip_prefix("mod_api::")
        .or_else(|| path.strip_prefix("crate::"))
        .unwrap_or(path);
    let id = format!("type:{path}");
    surfaces
        .iter()
        .flat_map(|s| &s.types)
        .find(|t| t.id == id || t.paths.iter().any(|p| p == path))
        .map_or(id, |t| t.id.clone())
}

fn shape(shape: &Option<String>) -> String {
    shape
        .as_ref()
        .map_or_else(|| "none".into(), |s| format!("`{s}`"))
}

/// What reaches a type whose layout changed, for its line.
fn reachers(by: &BTreeSet<String>) -> String {
    let named: Vec<String> = by.iter().take(3).cloned().collect();
    let more = by.len().saturating_sub(named.len());
    let mut text = named.join(", ");
    if more > 0 {
        text += &format!(" and {more} more");
    }
    text
}

/// Adds to `diff` the changes of the boundary from `old` to `new`, all breaking: a changed core,
/// an existing call gone or changed, and a layout change of an owned type an existing call or the
/// core reaches. `old_surface` and `new_surface` are the surfaces `diff` compared.
pub fn apply(
    diff: &mut Diff,
    old_surface: &Surface,
    new_surface: &Surface,
    old: &Boundary,
    new: &Boundary,
) {
    let surfaces = [new_surface, old_surface];
    let mut lines: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if old.core != new.core {
        lines.entry("core:wire".into()).or_default().push(format!(
            "the wire core changed (`{}` → `{}`): a transport value or control payload, a hand-written codec, the NBT tag ids or limits, or a prepared hook's ABI; {REFUSED}",
            old.core, new.core
        ));
    }
    // Owned types whose layout changed: their layouts, and what reaches them.
    type Layouts = BTreeMap<String, (Option<String>, Option<String>, BTreeSet<String>)>;
    let mut layouts = Layouts::new();
    let mut compare = |before: &[Owned], after: &[Owned], by: String| {
        for was in before {
            if let Some(now) = after.iter().find(|o| o.path == was.path)
                && now.shape != was.shape
            {
                layouts
                    .entry(was.path.clone())
                    .or_insert((was.shape.clone(), now.shape.clone(), BTreeSet::new()))
                    .2
                    .insert(by.clone());
            }
        }
    };
    let owned_paths =
        |owned: &[Owned]| -> BTreeSet<String> { owned.iter().map(|o| o.path.clone()).collect() };
    let changed_set = |key: &str, before: &[Owned], after: &[Owned]| {
        let (before, after) = (owned_paths(before), owned_paths(after));
        let list = |set: Vec<&String>| {
            set.iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        (before != after).then(|| {
            format!(
                "{key}: the owned types it reaches changed (gained: {}; lost: {}): {REFUSED}",
                list(after.difference(&before).collect()),
                list(before.difference(&after).collect()),
            )
        })
    };
    compare(&old.core_owned, &new.core_owned, "the wire core".into());
    if let Some(line) = changed_set("the wire core", &old.core_owned, &new.core_owned) {
        lines.entry("core:wire".into()).or_default().push(line);
    }
    let after: BTreeMap<&str, &Call> = new.calls.iter().map(|c| (c.key.as_str(), c)).collect();
    for call in &old.calls {
        let mut said = Vec::new();
        match after.get(call.key.as_str()) {
            None => said.push(format!("call `{}` is gone: {REFUSED}", call.key)),
            Some(now) => {
                if now.export != call.export {
                    said.push(format!(
                        "call `{}` is now {}: {REFUSED}",
                        call.key,
                        if now.export { "an export" } else { "an import" }
                    ));
                }
                if now.signature != call.signature {
                    said.push(format!(
                        "call `{}`: its signature hash changed (`{}` → `{}`), its arguments, answer or a borrowing record they reach: {REFUSED}",
                        call.key, call.signature, now.signature
                    ));
                }
                let key = format!("call `{}`", call.key);
                said.extend(changed_set(&key, &call.owned, &now.owned));
                compare(&call.owned, &now.owned, format!("`{}`", call.key));
            }
        }
        if !said.is_empty() {
            lines
                .entry(call_id(&call.key, surfaces))
                .or_default()
                .extend(said);
        }
    }
    for (path, (was, now, by)) in layouts {
        let id = type_id(&path, surfaces);
        let reached = format!("reached by {}, so {REFUSED}", reachers(&by));
        // The surface's own line for the same change says it affects wasm mods only.
        let mut raised = false;
        if let Some(change) = diff.changes.iter_mut().find(|c| c.id == id) {
            for line in change.what.iter_mut() {
                let wire = line.starts_with("wire layout:")
                    || line.starts_with("wire form:")
                    || line.starts_with("no longer crosses");
                if wire && line.ends_with("affects wasm mods only") {
                    let kept = line.trim_end_matches("affects wasm mods only");
                    *line = format!("{kept}{reached}");
                    raised = true;
                }
            }
            if raised {
                change.class = Class::Breaking;
            }
        }
        if !raised {
            lines.entry(id).or_default().push(format!(
                "wire layout: {} → {}: {reached}",
                shape(&was),
                shape(&now)
            ));
        }
    }
    for (id, said) in lines {
        match diff.changes.iter_mut().find(|c| c.id == id) {
            Some(change) => {
                change.what.extend(said);
                change.class = Class::Breaking;
            }
            None => diff.changes.push(Change {
                id,
                kind: ChangeKind::Changed,
                class: Class::Breaking,
                what: said,
                members: Vec::new(),
                notes: Vec::new(),
            }),
        }
    }
    diff.changes
        .sort_by(|a, b| b.class.cmp(&a.class).then_with(|| a.id.cmp(&b.id)));
    diff.verdict = match diff.changes.iter().map(|c| c.class).max() {
        None => Verdict::Identical,
        Some(Class::Additive) => Verdict::Additive,
        Some(Class::Conditional) => Verdict::Conditional,
        Some(Class::Breaking) => Verdict::Breaking,
    };
}

#[cfg(test)]
#[path = "boundary_tests.rs"]
mod tests;
