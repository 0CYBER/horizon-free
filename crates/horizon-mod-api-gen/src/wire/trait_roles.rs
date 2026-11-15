//! Discover call-table roles from one file per newly exposed trait.
//! This classifies calls only: value layouts still come exclusively from Wire.
#[cfg(test)]
#[path = "trait_roles/tests.rs"]
mod tests;
use std::{
    cell::Cell,
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

struct Roles {
    imports: Vec<&'static str>,
    exports: Vec<&'static str>,
}
thread_local! {
    static CURRENT: Cell<Option<&'static Roles>> = const { Cell::new(None) };
}
// Method descriptors already retain static owner names. Intern each distinct
// source surface once for the generation process; fixture roots can select
// different surfaces concurrently without contaminating one another.
type Key = (Vec<String>, Vec<String>);
static SURFACES: OnceLock<Mutex<BTreeMap<Key, &'static Roles>>> = OnceLock::new();

pub(super) fn imports() -> &'static [&'static str] {
    CURRENT.with(|current| {
        current
            .get()
            .map_or(super::IMPORTS, |roles| roles.imports.as_slice())
    })
}
pub(super) fn exports() -> &'static [&'static str] {
    CURRENT.with(|current| {
        current
            .get()
            .map_or(super::EXPORTS, |roles| roles.exports.as_slice())
    })
}
pub(super) fn select(imports: Vec<String>, exports: Vec<String>) {
    let mut surfaces = SURFACES.get_or_init(Mutex::default).lock().unwrap();
    let roles = *surfaces
        .entry((imports, exports))
        .or_insert_with_key(|(imports, exports)| {
            let intern = |names: &[String]| {
                names
                    .iter()
                    .map(|name| &*Box::leak(name.clone().into_boxed_str()))
                    .collect()
            };
            Box::leak(Box::new(Roles {
                imports: intern(imports),
                exports: intern(exports),
            }))
        });
    CURRENT.with(|current| current.set(Some(roles)));
}

pub(super) fn declared(root: &Path) -> Result<(Vec<String>, Vec<String>, Vec<PathBuf>), String> {
    let directory = root.join("crates/horizon-mod-vocab/src/wire/traits");
    let mut imports: Vec<_> = super::IMPORTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let mut exports: Vec<_> = super::EXPORTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    if !directory.exists() {
        return Ok((imports, exports, Vec::new()));
    }
    let mut paths = std::fs::read_dir(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut seen: HashSet<_> = imports.iter().chain(&exports).cloned().collect();
    for path in &paths {
        let role = path.extension().and_then(|value| value.to_str());
        let target = match role {
            Some("import") => &mut imports,
            Some("export") => &mut exports,
            Some("md") => continue,
            _ => {
                return Err(format!(
                    "{}: expected a .import or .export trait role",
                    path.display()
                ));
            }
        };
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("{}: trait name is not UTF-8", path.display()))?;
        syn::parse_str::<syn::Ident>(name)
            .map_err(|error| format!("{}: invalid trait name: {error}", path.display()))?;
        if !seen.insert(name.to_owned()) {
            return Err(format!(
                "{}: duplicate Wire trait role for {name}",
                path.display()
            ));
        }
        target.push(name.to_owned());
    }
    // Cargo must rerun when a new independent trait file is added or removed.
    paths.push(directory);
    Ok((imports, exports, paths))
}
