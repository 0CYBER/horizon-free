//! The game's source tree as the generator walks it: each crate's files with their module paths
//! and the `cfg`s they are compiled under, following `mod` declarations, `#[path]`s and inline
//! modules.

use mod_source::modules::{ModuleFile, walk_module_files};
use quote::ToTokens;
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use syn::Type;

/// The files of the game's module `module` ([`walk_module_files`]), each parsed with the game's
/// generated module lists expanded. With `every_build`, the replaceable functions: those every
/// build compiles; without, the internals, listed with the builds they exist in.
pub(crate) fn module_files(
    module: String,
    path: PathBuf,
    dir: PathBuf,
    every_build: bool,
) -> Result<Vec<ModuleFile>, String> {
    let key = (module.clone(), path.clone(), dir.clone(), every_build);
    if let Some(files) = WALKS.with(|walks| walks.borrow().as_ref()?.get(&key).cloned()) {
        return Ok(files);
    }
    let files = walk_module_files(
        module,
        path,
        dir,
        every_build,
        false,
        crate::game_modules::parse,
    )?;
    WALKS.with(|walks| {
        if let Some(walks) = walks.borrow_mut().as_mut() {
            walks.insert(key, files.clone());
        }
    });
    Ok(files)
}

type WalkKey = (String, PathBuf, PathBuf, bool);

thread_local! {
    static WALKS: RefCell<Option<HashMap<WalkKey, Vec<ModuleFile>>>> = const { RefCell::new(None) };
}

/// Runs `run` with [`module_files`] remembering each walk, for one generation whose sources don't
/// change while it runs (a build script's): it walks and parses the same module trees several
/// times, once to list what to track and again to read them. Never around a pass that edits the
/// files it walks, such as staging.
pub(crate) fn with_walk_cache<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(Option<HashMap<WalkKey, Vec<ModuleFile>>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            WALKS.with(|walks| *walks.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(WALKS.with(|walks| walks.replace(Some(HashMap::new()))));
    run()
}

/// The files of `horizon_client::game`; see [`module_files`] for `every_build`.
pub(crate) fn game_files(src: &Path, every_build: bool) -> Result<Vec<ModuleFile>, String> {
    module_files(
        "horizon_client::game".to_owned(),
        src.join("game.rs"),
        src.join("game"),
        every_build,
    )
}

/// The crate `krate`'s package folder and library root, from its manifest.
pub(crate) fn crate_root(root: &Path, krate: &str) -> Result<(PathBuf, PathBuf), String> {
    let package = root.join("crates").join(krate.replace('_', "-"));
    let manifest = package.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let manifest: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
    let lib = manifest
        .get("lib")
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str)
        .unwrap_or("src/lib.rs");
    let lib = package.join(lib);
    Ok((package, lib))
}

/// The files of the crate `krate` that every build compiles.
pub(crate) fn crate_files(root: &Path, krate: &str) -> Result<Vec<ModuleFile>, String> {
    let (_, lib) = crate_root(root, krate)?;
    let dir = lib.parent().expect("a source file has a folder").to_owned();
    module_files(krate.to_owned(), lib, dir, true)
}

/// A path without generic arguments or spaces, as listed paths and found ones are compared.
pub(crate) fn plain_path(path: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in path.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            c if depth == 0 && !c.is_whitespace() => out.push(c),
            _ => {}
        }
    }
    out
}

/// `r#type` as `type`.
pub(crate) fn unraw(ident: &syn::Ident) -> String {
    ident.to_string().trim_start_matches("r#").to_owned()
}

/// A type as a target names it: lifetimes as `'_`, no spaces.
pub(crate) fn owner(ty: &Type) -> String {
    struct EraseLifetimes;
    impl syn::visit_mut::VisitMut for EraseLifetimes {
        fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
            *lifetime = syn::Lifetime::new("'_", lifetime.span());
        }
    }
    let mut ty = ty.clone();
    syn::visit_mut::VisitMut::visit_type_mut(&mut EraseLifetimes, &mut ty);
    ty.to_token_stream().to_string().replace(' ', "")
}

/// The byte offset of `at` in `source`, whose line starts are `lines`.
pub(crate) fn offset(source: &str, lines: &[usize], at: proc_macro2::LineColumn) -> usize {
    let line = lines[at.line - 1];
    source[line..]
        .char_indices()
        .nth(at.column)
        .map_or(source.len(), |(i, _)| line + i)
}

/// A function marker shared by source-discovery passes.
pub(crate) fn marker<'a>(attrs: &'a [syn::Attribute], name: &str) -> Option<&'a syn::Attribute> {
    attrs
        .iter()
        .find(|attr| attr.path().segments.last().is_some_and(|s| s.ident == name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module whose file each target names with `cfg_attr(…, path = …)` is read from each
    /// target's file under its condition; it has no default file, and none is needed.
    #[test]
    fn per_target_module_paths_are_followed_and_need_no_default_file() {
        let dir = std::env::temp_dir().join(format!("points-cfg-attr-{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("control")).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "#[cfg_attr(unix, path = \"control/unix.rs\")]\n#[cfg_attr(windows, path = \"control/windows.rs\")]\nmod platform;\n",
        )
        .unwrap();
        std::fs::write(src.join("control/unix.rs"), "pub fn a() {}").unwrap();
        std::fs::write(src.join("control/windows.rs"), "pub fn b() {}").unwrap();
        let files = module_files("k".into(), src.join("lib.rs"), src.clone(), false).unwrap();
        let mut found: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.module.as_str(),
                    f.path.strip_prefix(&src).unwrap().to_owned(),
                    f.cfg.clone(),
                )
            })
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                ("k", "lib.rs".into(), vec![]),
                (
                    "k::platform",
                    "control/unix.rs".into(),
                    vec!["unix".to_owned()]
                ),
                (
                    "k::platform",
                    "control/windows.rs".into(),
                    vec!["windows".to_owned()]
                ),
            ]
        );
        // Every build compiles neither, so the point walk leaves the module out.
        let every = module_files("k".into(), src.join("lib.rs"), src, true).unwrap();
        assert_eq!(every.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Inside one generation a walk is read once; outside it, every walk reads the tree again.
    #[test]
    fn a_generation_reuses_its_module_walks_and_only_inside_its_scope() {
        let dir = std::env::temp_dir().join(format!("points-walk-cache-{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "mod a;\n").unwrap();
        std::fs::write(src.join("a.rs"), "pub fn a() {}").unwrap();
        let modules = |files: Vec<ModuleFile>| {
            let mut modules: Vec<_> = files.into_iter().map(|file| file.module).collect();
            modules.sort();
            modules
        };
        let walk = || module_files("k".into(), src.join("lib.rs"), src.clone(), true).unwrap();
        with_walk_cache(|| {
            assert_eq!(modules(walk()), ["k", "k::a"]);
            // The tree is read once: an edit during the generation is not seen.
            std::fs::write(src.join("lib.rs"), "").unwrap();
            assert_eq!(modules(walk()), ["k", "k::a"]);
        });
        assert_eq!(modules(walk()), ["k"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
