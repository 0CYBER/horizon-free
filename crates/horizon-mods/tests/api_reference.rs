//! docs/api/mods-reference.md, generated from the WIT contract and the manifest's capabilities, and a test that
//! fails when it is out of date. Update it with:
//! `HORIZON_WRITE_DOCS=1 cargo test -p horizon-mods --features pack --test api_reference`.
use horizon_mods::manifest::{APIS, Capability};
use std::{fmt::Write, path::Path};
use wit_parser::{Docs, Function, Handle, Resolve, Type, TypeDefKind, WorldItem};

const WIT: &str = "crates/horizon-mod-sdk/wit/horizon.wit";
const DOC: &str = "docs/api/mods-reference.md";

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// A doc comment on one line, for a table cell.
fn cell(docs: &Docs) -> String {
    let text = docs.contents.as_deref().unwrap_or("").trim();
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

/// A type as WIT spells it.
fn ty(resolve: &Resolve, t: &Type) -> String {
    let id = match t {
        Type::Bool => return "bool".into(),
        Type::U8 => return "u8".into(),
        Type::U16 => return "u16".into(),
        Type::U32 => return "u32".into(),
        Type::U64 => return "u64".into(),
        Type::S8 => return "s8".into(),
        Type::S16 => return "s16".into(),
        Type::S32 => return "s32".into(),
        Type::S64 => return "s64".into(),
        Type::F32 => return "f32".into(),
        Type::F64 => return "f64".into(),
        Type::Char => return "char".into(),
        Type::String => return "string".into(),
        Type::ErrorContext => return "error-context".into(),
        Type::Id(id) => *id,
    };
    match &resolve.types[id].name {
        Some(name) => name.clone(),
        None => definition(resolve, &resolve.types[id].kind),
    }
}

/// What an anonymous type, or the right-hand side of a `type` alias, spells.
fn definition(resolve: &Resolve, kind: &TypeDefKind) -> String {
    let of = |t: &Option<Type>| t.as_ref().map_or("_".into(), |t| ty(resolve, t));
    match kind {
        TypeDefKind::Option(t) => format!("option<{}>", ty(resolve, t)),
        TypeDefKind::List(t) => format!("list<{}>", ty(resolve, t)),
        TypeDefKind::FixedLengthList(t, n) => format!("list<{}, {n}>", ty(resolve, t)),
        TypeDefKind::Result(r) => match (&r.ok, &r.err) {
            (None, None) => "result".into(),
            (ok, None) => format!("result<{}>", of(ok)),
            (ok, err) => format!("result<{}, {}>", of(ok), of(err)),
        },
        TypeDefKind::Tuple(t) => {
            let items: Vec<_> = t.types.iter().map(|t| ty(resolve, t)).collect();
            format!("tuple<{}>", items.join(", "))
        }
        TypeDefKind::Future(t) => t
            .as_ref()
            .map_or("future".into(), |t| format!("future<{}>", ty(resolve, t))),
        TypeDefKind::Stream(t) => t
            .as_ref()
            .map_or("stream".into(), |t| format!("stream<{}>", ty(resolve, t))),
        TypeDefKind::Handle(Handle::Own(t)) => ty(resolve, &Type::Id(*t)),
        TypeDefKind::Handle(Handle::Borrow(t)) => format!("borrow<{}>", ty(resolve, &Type::Id(*t))),
        TypeDefKind::Type(t) => ty(resolve, t),
        other => panic!(
            "{WIT}: an anonymous {} the reference can't spell",
            other.as_str()
        ),
    }
}

fn signature(resolve: &Resolve, f: &Function) -> String {
    let params: Vec<_> = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, ty(resolve, &p.ty)))
        .collect();
    let result = f
        .result
        .as_ref()
        .map_or(String::new(), |t| format!(" -> {}", ty(resolve, t)));
    format!("func({}){result}", params.join(", "))
}

/// Who may use an interface: the capability that grants it, or what it is.
fn access(interface: &str, exported: bool) -> String {
    if exported {
        return "exported by the mod: the game calls it".into();
    }
    if interface == "log" {
        return "always granted".into();
    }
    match Capability::ALL.iter().find(|c| c.interface() == interface) {
        Some(c) => format!("capability `{c}`"),
        None => "not granted by any capability".into(),
    }
}

fn markdown() -> String {
    let mut resolve = Resolve::new();
    let package = resolve
        .push_file(root().join(WIT))
        .expect("the WIT contract parses");
    let package = &resolve.packages[package];
    let name = &package.name;
    let mut out = String::new();
    let w = &mut out;
    writeln!(w, "<!-- Generated from {WIT} and the manifest's capabilities by crates/horizon-mods/tests/api_reference.rs; a test fails when this file is out of date. Update it with: HORIZON_WRITE_DOCS=1 cargo test -p horizon-mods --features pack --test api_reference -->").unwrap();
    writeln!(w, "# Mod API reference\n").unwrap();
    writeln!(
        w,
        "The contract between Horizon and its mods: the WIT package `{name}`, every interface, type, function and hook in it, \
         and the capabilities a manifest may list. The guide is [mods.md](../mods.md); stability and versions: \
         [stability.md](stability.md). Manifest `api` lines this game serves: {}.\n",
        APIS.map(|a| format!("`{a}`")).join(", ")
    )
    .unwrap();

    writeln!(w, "## Capabilities\n").unwrap();
    writeln!(
        w,
        "| In `capabilities` | Grants | The player reads \"May …\" |\n| --- | --- | --- |"
    )
    .unwrap();
    for c in Capability::ALL {
        let grants = if package.interfaces.contains_key(c.interface()) {
            format!("[`{0}`](#{0})", c.interface())
        } else {
            "data files (`assets/`, `data/`), no interface".into()
        };
        writeln!(w, "| `{c}` | {grants} | {} |", c.describe()).unwrap();
    }

    let mut exported = Vec::new();
    for (world_name, &world) in &package.worlds {
        let world = &resolve.worlds[world];
        let prefix = format!("{}:{}/", name.namespace, name.name);
        let names = |keys: Vec<&wit_parser::WorldKey>| {
            keys.into_iter()
                .map(|key| {
                    let full = resolve.name_world_key(key);
                    let short = full.trim_start_matches(&prefix);
                    format!("`{}`", short.split('@').next().unwrap_or(short))
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        writeln!(w, "\n## World `{world_name}`\n").unwrap();
        if world.docs.contents.is_some() {
            writeln!(w, "{}\n", cell(&world.docs)).unwrap();
        }
        writeln!(
            w,
            "- Imports (what the game offers): {}",
            names(world.imports.keys().collect())
        )
        .unwrap();
        writeln!(
            w,
            "- Exports (what the mod provides): {}",
            names(world.exports.keys().collect())
        )
        .unwrap();
        for item in world.exports.values() {
            if let WorldItem::Interface { id, .. } = item {
                exported.push(*id);
            }
        }
    }

    for (interface_name, &id) in &package.interfaces {
        let interface = &resolve.interfaces[id];
        writeln!(w, "\n## `{interface_name}`\n").unwrap();
        writeln!(
            w,
            "Access: {}.\n",
            access(interface_name, exported.contains(&id))
        )
        .unwrap();
        if interface.docs.contents.is_some() {
            writeln!(w, "{}\n", cell(&interface.docs)).unwrap();
        }
        for (type_name, &type_id) in &interface.types {
            let def = &resolve.types[type_id];
            writeln!(w, "### {} `{type_name}`\n", def.kind.as_str()).unwrap();
            if def.docs.contents.is_some() {
                writeln!(w, "{}\n", cell(&def.docs)).unwrap();
            }
            match &def.kind {
                TypeDefKind::Record(r) => {
                    writeln!(w, "| Field | Type | |\n| --- | --- | --- |").unwrap();
                    for f in &r.fields {
                        writeln!(
                            w,
                            "| `{}` | `{}` | {} |",
                            f.name,
                            ty(&resolve, &f.ty),
                            cell(&f.docs)
                        )
                        .unwrap();
                    }
                }
                TypeDefKind::Enum(e) => {
                    writeln!(w, "| Case | |\n| --- | --- |").unwrap();
                    for c in &e.cases {
                        writeln!(w, "| `{}` | {} |", c.name, cell(&c.docs)).unwrap();
                    }
                }
                TypeDefKind::Variant(v) => {
                    writeln!(w, "| Case | Payload | |\n| --- | --- | --- |").unwrap();
                    for c in &v.cases {
                        let payload =
                            c.ty.as_ref()
                                .map_or(String::new(), |t| format!("`{}`", ty(&resolve, t)));
                        writeln!(w, "| `{}` | {payload} | {} |", c.name, cell(&c.docs)).unwrap();
                    }
                }
                TypeDefKind::Flags(f) => {
                    writeln!(w, "| Flag | |\n| --- | --- |").unwrap();
                    for flag in &f.flags {
                        writeln!(w, "| `{}` | {} |", flag.name, cell(&flag.docs)).unwrap();
                    }
                }
                TypeDefKind::Resource => {
                    writeln!(w, "A resource: the game owns it, the mod holds a handle.").unwrap()
                }
                kind => writeln!(w, "`type {type_name} = {}`", definition(&resolve, kind)).unwrap(),
            }
            writeln!(w).unwrap();
        }
        if !interface.functions.is_empty() {
            writeln!(
                w,
                "| Function | Signature | What it does |\n| --- | --- | --- |"
            )
            .unwrap();
            for (fn_name, f) in &interface.functions {
                writeln!(
                    w,
                    "| `{fn_name}` | `{}` | {} |",
                    signature(&resolve, f),
                    cell(&f.docs)
                )
                .unwrap();
            }
        }
    }
    out
}

#[test]
fn the_mod_api_reference_is_up_to_date() {
    let path = root().join(DOC);
    let want = markdown();
    if std::env::var_os("HORIZON_WRITE_DOCS").is_some() {
        std::fs::write(&path, &want).unwrap();
    }
    let have = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        have == want,
        "{DOC} is out of date: HORIZON_WRITE_DOCS=1 cargo test -p horizon-mods --features pack --test api_reference"
    );
}

#[test]
fn every_function_is_documented() {
    let mut resolve = Resolve::new();
    resolve.push_file(root().join(WIT)).unwrap();
    for (_, interface) in resolve.interfaces.iter() {
        for (name, f) in &interface.functions {
            assert!(
                f.docs.contents.is_some(),
                "{WIT}: `{name}` needs a doc comment: the reference shows it to mod authors"
            );
        }
    }
}

#[test]
fn every_capability_with_an_interface_is_in_the_contract() {
    let mut resolve = Resolve::new();
    let package = resolve.push_file(root().join(WIT)).unwrap();
    let interfaces = &resolve.packages[package].interfaces;
    for c in Capability::ALL {
        if c != Capability::Registries {
            assert!(
                interfaces.contains_key(c.interface()),
                "capability {c} grants `{}`, missing from {WIT}",
                c.interface()
            );
        }
    }
}
