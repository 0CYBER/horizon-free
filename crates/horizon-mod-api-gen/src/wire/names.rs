//! How a call is known on both sides of the boundary: by its name, never its position, so a
//! method added anywhere in the API leaves every other call as it was (#18253).
//!
//! - An import is its own wasm import, `<owner>::<name>#<signature>` (`held-` first for a
//!   method of a handler another mod gave the game). The signature hash covers the receiver,
//!   the argument and answer types and, for a type that borrows, the shape the generator
//!   writes for it. A missing or changed import fails linking, naming the method.
//! - An export's id is a hash of its name, the same in every build.
//! - [`layouts`] is the table both sides compute at run time: each call's signature hash and
//!   the load-phase access the game checks before serving it ([`access_checks`]), mixed with
//!   the `Wire` layout of every owned type it reaches, which the generator can't see. The game
//!   compares the mod's table with its own when the mod loads.
use super::ty::{BorrowedShape, Ty, Types};
use super::{Method, exports};
use std::fmt::Write as _;

fn fnv(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// `ty` written by what crosses, never by the generator's numbering of types, which an added
/// type shifts. A type that borrows is written out with its generated shape.
fn canon(types: &Types<'_>, ty: &Ty, out: &mut String, seen: &mut Vec<usize>) {
    let each = |out: &mut String, items: &[&Ty], seen: &mut Vec<usize>| {
        for item in items {
            canon(types, item, out, seen);
            out.push(',');
        }
    };
    match ty {
        Ty::Unit => out.push_str("()"),
        Ty::Prim(name) | Ty::Wide(name) => out.push_str(name),
        Ty::Duration => out.push_str("Duration"),
        Ty::Str(kind) => write!(out, "str:{kind:?}").unwrap(),
        Ty::List(item, kind) => {
            write!(out, "list:{kind:?}<").unwrap();
            each(out, &[item], seen);
            out.push('>');
        }
        Ty::Option(item) => {
            out.push_str("option<");
            each(out, &[item], seen);
            out.push('>');
        }
        Ty::Result(ok, error) => {
            out.push_str("result<");
            each(out, &[ok, error], seen);
            out.push('>');
        }
        Ty::Tuple(items) => {
            out.push('(');
            each(out, &items.iter().collect::<Vec<_>>(), seen);
            out.push(')');
        }
        Ty::Array(item, len) => {
            out.push('[');
            each(out, &[item], seen);
            write!(out, ";{len}]").unwrap();
        }
        Ty::Ref(item, mutable) => {
            out.push_str(if *mutable { "&mut " } else { "&" });
            each(out, &[item], seen);
        }
        Ty::Boxed(item) => {
            out.push_str("box<");
            each(out, &[item], seen);
            out.push('>');
        }
        Ty::Shared(item) => {
            out.push_str("arc<");
            each(out, &[item], seen);
            out.push('>');
        }
        Ty::Dyn(name, mutable) => write!(out, "dyn:{name}:{mutable}").unwrap(),
        Ty::Handler(name, boxed) => write!(out, "handler:{name}:{boxed}").unwrap(),
        Ty::Named(def) => {
            let def_ty = &types.defs[*def];
            out.push_str(&def_ty.rust);
            if def_ty.lifetimes == 0 || seen.contains(def) {
                return;
            }
            seen.push(*def);
            let fields = |out: &mut String, fields: &[super::ty::Field], seen: &mut Vec<usize>| {
                for field in fields {
                    write!(out, "{}:", field.rust).unwrap();
                    canon(types, &field.ty, out, seen);
                    out.push(',');
                }
            };
            write!(out, "{{lifetimes:{};", def_ty.lifetimes).unwrap();
            match &def_ty.borrowed {
                BorrowedShape::Record(list) => {
                    out.push_str("record:");
                    fields(out, list, seen);
                }
                BorrowedShape::TupleRecord(list) => {
                    out.push_str("tuple:");
                    fields(out, list, seen);
                }
                BorrowedShape::Variant(cases) => {
                    out.push_str("variant:");
                    for case in cases {
                        write!(out, "{}(", case.rust).unwrap();
                        each(out, &case.payload.iter().collect::<Vec<_>>(), seen);
                        out.push_str("),");
                    }
                }
                BorrowedShape::None => out.push_str("owned"),
            }
            if let Some(append) = &def_ty.append {
                write!(out, ";append:{}:{}:", append.field, append.equal).unwrap();
                fields(out, &append.fields, seen);
                canon(types, &append.element, out, seen);
            }
            if let Some(projection) = &def_ty.projection {
                out.push_str(";projection:");
                fields(out, projection, seen);
            }
            if let Some(view) = &def_ty.view {
                write!(out, ";view:{}:{}:", view.field, view.mutable).unwrap();
                canon(types, &view.pointee, out, seen);
                out.push(':');
                canon(types, &view.snapshot, out, seen);
            }
            out.push('}');
            seen.pop();
        }
    }
}

/// The hash of what `m` carries as the generator sees it: its receiver, its arguments by name
/// and type in order (two swapped arguments of one type mean another call), its answer type,
/// and the generated shape of each borrowing type among them.
pub(super) fn signature(types: &Types<'_>, m: &Method) -> u64 {
    let mut text = format!("receiver:{:?};", m.receiver);
    let mut seen = Vec::new();
    for (name, ty) in &m.params {
        write!(text, "{name}:").unwrap();
        canon(types, ty, &mut text, &mut seen);
        text.push(',');
    }
    text.push_str("->");
    canon(types, &m.ret, &mut text, &mut seen);
    fnv(&text)
}

/// The accessor that hands `m`'s trait over (`REACHED`): the generated host checks its access
/// before `m`'s own.
pub(super) fn frame_accessor_of<'a>(m: &Method, dropped: &'a [Method]) -> Option<&'a Method> {
    super::REACHED
        .iter()
        .find(|(owner, _, _)| *owner == m.owner)
        .filter(|_| m.owner != "ConfigFiles")
        .and_then(|_| {
            dropped.iter().find(|candidate| {
                super::frame_accessor(candidate)
                    && matches!(&candidate.ret, Ty::Dyn(name, _) if name == m.owner)
            })
        })
}

/// The access the generated host checks before serving file root `source::accessor`.
pub(super) fn root_access<'a>(
    source: &str,
    accessor: &str,
    methods: &'a [Method],
    dropped: &'a [Method],
) -> &'a str {
    dropped
        .iter()
        .chain(methods)
        .find(|method| method.owner == source && method.name == accessor)
        .and_then(|method| method.access.as_deref())
        .unwrap_or("WorldOnly")
}

/// Every load-phase access check the generated host makes before serving `m`, in order: each
/// file root's for `ConfigFiles`, the accessor's that hands its trait over, then its own. A
/// mod built before one changed would call into a phase that now refuses it.
pub(super) fn access_checks(m: &Method, methods: &[Method], dropped: &[Method]) -> String {
    let mut text = String::new();
    if m.owner == "ConfigFiles" {
        for (root, source, accessor) in super::FILE_ROOTS {
            let level = root_access(source, accessor, methods, dropped);
            write!(text, "root {root}:{level};").unwrap();
        }
    }
    for checked in frame_accessor_of(m, dropped).into_iter().chain([m]) {
        write!(text, "{}:{:?};", checked.name, checked.access).unwrap();
    }
    text
}

/// `m`'s [`signature`] with its [`access_checks`]: its hash in [`layouts`] before the layouts of
/// the types it reaches.
fn checked_signature(types: &Types<'_>, m: &Method, methods: &[Method], dropped: &[Method]) -> u64 {
    fnv(&format!(
        "{:016x}:{}",
        signature(types, m),
        access_checks(m, methods, dropped)
    ))
}

/// The call's key in [`layouts`] and in its import name: `owner::name`, with `held-` before a
/// method the mod calls on a handler another mod gave the game.
pub(super) fn key(m: &Method, held: bool) -> String {
    format!("{}{}::{}", if held { "held-" } else { "" }, m.owner, m.name)
}

/// The wasm import (module `horizon`) a guest calls `m` through.
pub(super) fn import_name(types: &Types<'_>, m: &Method, held: bool) -> String {
    format!("{}#{:016x}", key(m, held), signature(types, m))
}

/// An export's id: its name's hash, below the point listeners' range (`0x8000_0000..`) and the
/// reserved transport ids (`0xFFFF_FFxx`), so the same export has the same id in every build.
pub(super) fn export_id(owner: &str, name: &str) -> u32 {
    (fnv(&format!("{owner}::{name}")) as u32) & 0x3FFF_FFFF
}

/// Refuses a surface where two exports share an id, or two imports a name: rename one of them.
pub(super) fn check_names(methods: &[Method]) -> Result<(), String> {
    let mut seen = std::collections::HashMap::new();
    for m in methods.iter().filter(|m| exports().contains(&m.owner)) {
        let id = export_id(m.owner, &m.name);
        if let Some(other) = seen.insert(id, key(m, false))
            && other != key(m, false)
        {
            return Err(format!(
                "exports {other} and {} share the wire id {id:#x}: rename one",
                key(m, false)
            ));
        }
    }
    // The game links an import by its key: two calls under one key couldn't both be linked.
    let mut keys = std::collections::HashSet::new();
    let imports = super::calls::imports(methods)
        .into_iter()
        .map(|m| key(m, false));
    let held = super::calls::held_methods(methods)
        .into_iter()
        .map(|m| key(m, true));
    for key in imports.chain(held) {
        if !keys.insert(key.clone()) {
            return Err(format!("two imports are both {key}: rename one"));
        }
    }
    Ok(())
}

/// The owned types a call reaches, whose `Wire::layout` the generator can't see: through its
/// arguments and answer, and through the fields of each borrowing type among them.
fn reached(types: &Types<'_>, ty: &Ty, found: &mut Vec<usize>, seen: &mut Vec<usize>) {
    match ty {
        Ty::List(item, _)
        | Ty::Option(item)
        | Ty::Array(item, _)
        | Ty::Ref(item, _)
        | Ty::Boxed(item)
        | Ty::Shared(item) => reached(types, item, found, seen),
        Ty::Result(ok, error) => {
            reached(types, ok, found, seen);
            reached(types, error, found, seen);
        }
        Ty::Tuple(items) => items.iter().for_each(|t| reached(types, t, found, seen)),
        Ty::Named(def) if types.defs[*def].lifetimes == 0 => {
            if !found.contains(def) {
                found.push(*def);
            }
        }
        Ty::Named(def) => {
            if seen.contains(def) {
                return;
            }
            seen.push(*def);
            let def_ty = &types.defs[*def];
            let mut tys: Vec<&Ty> = Vec::new();
            match &def_ty.borrowed {
                BorrowedShape::Record(list) | BorrowedShape::TupleRecord(list) => {
                    tys.extend(list.iter().map(|f| &f.ty))
                }
                BorrowedShape::Variant(cases) => tys.extend(cases.iter().flat_map(|c| &c.payload)),
                BorrowedShape::None => {}
            }
            if let Some(append) = &def_ty.append {
                tys.extend(append.fields.iter().map(|f| &f.ty));
                tys.push(&append.element);
            }
            if let Some(projection) = &def_ty.projection {
                tys.extend(projection.iter().map(|f| &f.ty));
            }
            if let Some(view) = &def_ty.view {
                tys.push(&view.pointee);
                tys.push(&view.snapshot);
            }
            for ty in tys {
                reached(types, ty, found, seen);
            }
        }
        _ => {}
    }
}

/// An export's hash: its signature's and its id's, so an id that changes for the same name
/// (another mask or hash) refuses mods built before.
fn export_hash(signature: u64, id: u32) -> u64 {
    fnv(&format!("{signature:016x}:{id:08x}"))
}

/// What a subscription export's answer crosses as, where the generated code converts its Rust
/// return (`HookId`s, whose layout grows with every hook) to names (`calls::dispatch`).
fn answer_crossing(m: &Method) -> Option<&'static str> {
    if !super::SUBSCRIBED
        .iter()
        .any(|(owner, name)| *owner == m.owner && *name == m.name)
    {
        return None;
    }
    Some(if m.owner == "Hooks" {
        "crate::wire::SubscribedAnswer"
    } else {
        "crate::wire::SubscriptionAnswer"
    })
}

/// Every call both sides know, as `(key, signature hash, owned types reached, export)`: the
/// imports, the methods of held handlers and the exports.
fn calls(
    types: &Types<'_>,
    methods: &[Method],
    dropped: &[Method],
) -> Vec<(String, u64, Vec<usize>, bool)> {
    let imports = super::calls::imports(methods)
        .into_iter()
        .map(|m| (m, false, false));
    let held = super::calls::held_methods(methods)
        .into_iter()
        .map(|m| (m, true, false));
    let own = methods
        .iter()
        .filter(|m| exports().contains(&m.owner))
        .map(|m| (m, false, true));
    imports
        .chain(held)
        .chain(own)
        .map(|(m, held, export)| {
            let mut found = Vec::new();
            let ret = answer_crossing(m).is_none().then_some(&m.ret);
            for ty in m.params.iter().map(|(_, ty)| ty).chain(ret) {
                reached(types, ty, &mut found, &mut Vec::new());
            }
            found.sort_by(|a, b| types.defs[*a].rust.cmp(&types.defs[*b].rust));
            let signature = checked_signature(types, m, methods, dropped);
            let signature = if export {
                export_hash(signature, export_id(m.owner, &m.name))
            } else {
                signature
            };
            (key(m, held), signature, found, export)
        })
        .collect()
}

/// `call_layouts()`, for the crate that defines `Wire`: each call's key and its whole hash, the
/// signature hash mixed with the run-time layout of each owned type it reaches in name order.
/// Each type's layout is computed once.
pub(super) fn layouts(types: &Types<'_>, methods: &[Method], dropped: &[Method]) -> String {
    let calls = calls(types, methods, dropped);
    let mut used: Vec<usize> = calls
        .iter()
        .flat_map(|(_, _, d, _)| d.iter().copied())
        .collect();
    used.sort_unstable();
    used.dedup();
    let slot = |def: usize| used.binary_search(&def).unwrap();
    let mut code = String::from(
        "/// Every call of this build by its name, with its signature and the wire layout of each\n/// owned type it reaches, and whether it is an export: what the game compares with a mod's own\n/// when it loads.\n#[doc(hidden)]\npub fn call_layouts() -> ::std::vec::Vec<(&'static str, u64, bool)> {\nlet d: [u64; ",
    );
    write!(code, "{}] = [", used.len()).unwrap();
    for def in &used {
        write!(
            code,
            "<{} as crate::wire::Wire>::layout(&mut vec![]),",
            types.defs[*def].rust.replace("mod_api::", "crate::")
        )
        .unwrap();
    }
    code.push_str("];\nlet m = crate::wire::layout_mix;\nvec![\n");
    let crossing: std::collections::HashMap<String, &str> = methods
        .iter()
        .filter_map(|m| Some((key(m, false), answer_crossing(m)?)))
        .collect();
    for (key, signature, found, export) in &calls {
        let reached: String = found
            .iter()
            .map(|def| format!("d[{}],", slot(*def)))
            .collect();
        let answer = match crossing.get(key).filter(|_| *export) {
            Some(ty) => format!("<{ty} as crate::wire::Wire>::layout(&mut vec![]),"),
            None => String::new(),
        };
        writeln!(
            code,
            "({key:?},[{reached}{answer}].into_iter().fold({signature:#x},m),{export}),"
        )
        .unwrap();
    }
    code.push_str("]\n}\n");
    code
}

#[cfg(test)]
mod tests {
    use super::super::Method;
    use super::super::ty::{Ty, Types};
    use super::*;

    fn method(owner: &'static str, name: &str, param: Ty) -> Method {
        Method {
            owner,
            via: None,
            name: name.to_owned(),
            access: None,
            receiver: Some(false),
            params: vec![("value".to_owned(), param)],
            ret: Ty::Prim("i64"),
        }
    }

    fn i32() -> Ty {
        Ty::Prim("i32")
    }

    /// Positions shift when a method is added mid-trait; names don't. A mod built before the
    /// addition still finds every import and export it knows under the same name and id.
    #[test]
    fn a_method_added_mid_trait_moves_no_other_call() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let types = Types::new(crate::index::Index::new(&root, &[])).unwrap();
        let build = |added: bool| {
            let mut methods = vec![
                method("Backend", "first", i32()),
                method("Backend", "last", i32()),
                method("Hooks", "first_hook", i32()),
                method("Hooks", "last_hook", i32()),
            ];
            if added {
                methods.insert(1, method("Backend", "middle", i32()));
                methods.insert(3, method("Hooks", "middle_hook", i32()));
            }
            methods
        };
        let (before, after) = (build(false), build(true));
        let imports = |methods: &[Method]| -> Vec<String> {
            methods
                .iter()
                .filter(|m| !exports().contains(&m.owner))
                .map(|m| import_name(&types, m, false))
                .collect()
        };
        let exported = |methods: &[Method]| -> Vec<(String, u32)> {
            methods
                .iter()
                .filter(|m| exports().contains(&m.owner))
                .map(|m| (key(m, false), export_id(m.owner, &m.name)))
                .collect()
        };
        let position = |methods: &[Method], name: &str| methods.iter().position(|m| m.name == name);
        assert_ne!(position(&before, "last"), position(&after, "last"));
        for name in imports(&before) {
            assert!(imports(&after).contains(&name), "{name} moved");
        }
        for export in exported(&before) {
            assert!(exported(&after).contains(&export), "{export:?} moved");
        }
        check_names(&after).unwrap();

        // A changed signature changes that call's import name and no other.
        let mut changed = build(false);
        changed[1].params[0].1 = Ty::Prim("i64");
        let (old, new) = (imports(&before), imports(&changed));
        assert_eq!(old[0], new[0]);
        assert_ne!(old[1], new[1]);
        assert_eq!(key(&before[1], false), key(&changed[1], false));
    }

    /// `entity_start_riding(entity, vehicle)` with its two arguments swapped is another call:
    /// a mod built against one order must not link against the other.
    /// An export's hash covers its id: an id changed for the same name and signature (another
    /// mask) refuses mods built before.
    #[test]
    fn an_exports_hash_covers_its_id() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let types = Types::new(crate::index::Index::new(&root, &[])).unwrap();
        let started = method("Hooks", "server_started", i32());
        let (_, hash, _, export) = calls(&types, std::slice::from_ref(&started), &[])
            .into_iter()
            .find(|(key, ..)| key == "Hooks::server_started")
            .unwrap();
        let id = export_id("Hooks", "server_started");
        assert!(export);
        let signature = checked_signature(&types, &started, &[], &[]);
        assert_eq!(hash, export_hash(signature, id));
        assert_ne!(hash, export_hash(signature, id ^ 1));
    }

    /// A subscription export is hashed by the names it sends, not by `HookId`'s layout, which
    /// a hook added to the game changes.
    #[test]
    fn subscriptions_hash_what_they_send() {
        let code = layouts(
            &Types::new(crate::index::Index::new(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
                &[],
            ))
            .unwrap(),
            &[method("Hooks", "subscribed", i32())],
            &[],
        );
        assert!(code.contains("crate::wire::SubscribedAnswer as crate::wire::Wire"));
    }

    /// The access the game checks before serving a call is part of that call's hash, and of no
    /// other's: a mod importing a call whose access changed is refused, and the rest load. Its
    /// import name stays the same, so the refusal names the call. An accessor that hands a
    /// trait over counts for every method of that trait.
    #[test]
    fn a_calls_access_is_part_of_its_hash() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let types = Types::new(crate::index::Index::new(&root, &[])).unwrap();
        let build = |own: &str, accessor: &str| {
            let mut value = method("Host", "data_map_value", i32());
            value.access = Some(own.to_owned());
            let mut levels = method("ServerHost", "levels", i32());
            levels.params.clear();
            levels.ret = Ty::Dyn("Levels".to_owned(), false);
            levels.access = Some(accessor.to_owned());
            let methods = vec![
                value,
                method("Host", "other", i32()),
                method("Levels", "level_time", i32()),
            ];
            (methods, vec![levels])
        };
        let hashes = |(methods, dropped): (Vec<Method>, Vec<Method>)| {
            let table: std::collections::HashMap<String, u64> = calls(&types, &methods, &dropped)
                .into_iter()
                .map(|(key, hash, ..)| (key, hash))
                .collect();
            let names: Vec<String> = methods
                .iter()
                .map(|m| import_name(&types, m, false))
                .collect();
            (
                ["Host::data_map_value", "Host::other", "Levels::level_time"].map(|key| table[key]),
                names,
            )
        };
        let (before, names) = hashes(build("PublishedResources", "WorldOnly"));
        let (own, own_names) = hashes(build("WorldOnly", "WorldOnly"));
        assert_ne!(before[0], own[0]);
        assert_eq!(before[1..], own[1..]);
        assert_eq!(names, own_names);
        let (reached, _) = hashes(build("PublishedResources", "PublishedResources"));
        assert_eq!(before[..2], reached[..2]);
        assert_ne!(before[2], reached[2]);
    }

    #[test]
    fn swapped_arguments_of_one_type_change_the_call() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let types = Types::new(crate::index::Index::new(&root, &[])).unwrap();
        let mut riding = method("BackendMut", "entity_start_riding", i32());
        riding.params = vec![("entity".to_owned(), i32()), ("vehicle".to_owned(), i32())];
        let before = import_name(&types, &riding, false);
        riding.params.reverse();
        assert_ne!(before, import_name(&types, &riding, false));
    }
}
