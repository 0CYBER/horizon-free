//! A crate's source tree as the tools walk it: each file with its module path and the `cfg`s it
//! is compiled under, following `mod` declarations, `#[path]`s and inline modules.

use quote::ToTokens;
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::Item;

/// A source file of a crate, with its module path and the `cfg`s it is compiled under.
#[derive(Clone)]
pub struct ModuleFile {
    pub module: String,
    pub path: PathBuf,
    /// The conditions of the `mod` declarations that lead to it, outermost first.
    pub cfg: Vec<String>,
}

/// The `cfg` conditions in `attrs`, as written, except `cfg(test)`.
pub fn conditions(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") && !test_only(std::slice::from_ref(attr)))
        .filter_map(|attr| attr.parse_args::<syn::Meta>().ok())
        .map(|meta| meta.to_token_stream().to_string())
        .collect()
}

/// Whether an item is compiled only into tests, explicitly or by `#[test]`/`#[bench]`.
pub fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || attr.path().is_ident("bench")
            || (attr.path().is_ident("cfg")
                && matches!(attr.parse_args::<syn::Meta>(), Ok(syn::Meta::Path(path)) if path.is_ident("test")))
    })
}

/// A module file still to read: its module path, file, children's folder and conditions.
type Pending = (String, PathBuf, PathBuf, Vec<String>);

/// The files of the module `module`, read from `path` (its children in `dir`), following `mod`
/// declarations (and `#[path]`s, and inline modules that declare files), each read with `parse`.
/// With `every_build`, a module behind a `cfg` is left out with everything under it; without, it
/// is kept with its conditions. Unless `include_tests`, test-only modules are left out, including
/// files gated by inner attributes.
pub fn walk_module_files(
    module: String,
    path: PathBuf,
    dir: PathBuf,
    every_build: bool,
    include_tests: bool,
    parse: impl Fn(&Path, &str) -> Result<syn::File, String>,
) -> Result<Vec<ModuleFile>, String> {
    let mut files = Vec::new();
    let mut pending = vec![(module, path, dir, Vec::new())];
    while let Some((module, path, dir, cfg)) = pending.pop() {
        // A declared module with no file is one another target builds (its file named by a
        // `cfg_attr` path this walk already followed): nothing here compiles from it.
        if !path.is_file() {
            continue;
        }
        let source = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file = parse(&path, &source)?;
        if (!include_tests && test_only(&file.attrs)) || (every_build && gated(&file.attrs)) {
            continue;
        }
        let cfg = cfg
            .into_iter()
            .chain(conditions(&file.attrs))
            .collect::<Vec<_>>();
        let here = path
            .parent()
            .expect("a source file has a folder")
            .to_owned();
        children(
            &module,
            &file.items,
            &here,
            &dir,
            &here,
            &cfg,
            every_build,
            include_tests,
            &mut pending,
        );
        files.push(ModuleFile { module, path, cfg });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Queues the module files `items` declare, looking inside inline modules too: a file module
/// declared in `mod a { mod b; }` lives in `dir/a/b.rs`. A `#[path]` is relative to `here`.
fn children(
    module: &str,
    items: &[Item],
    here: &Path,
    dir: &Path,
    include_directory: &Path,
    cfg: &[String],
    every_build: bool,
    include_tests: bool,
    pending: &mut Vec<Pending>,
) {
    for item in items {
        if let Item::Macro(included) = item
            && included.mac.path.is_ident("include")
            && (include_tests || !test_only(&included.attrs))
            && !(every_build && gated(&included.attrs))
            && let Ok(relative) = syn::parse2::<syn::LitStr>(included.mac.tokens.clone())
        {
            pending.push((
                module.to_owned(),
                include_directory.join(relative.value()),
                dir.to_owned(),
                cfg.to_vec(),
            ));
        }
        let Item::Mod(declared) = item else { continue };
        if (!include_tests && test_only(&declared.attrs)) || (every_build && gated(&declared.attrs))
        {
            continue;
        }
        let name = declared.ident.to_string();
        let child = format!("{module}::{name}");
        let cfg: Vec<String> = cfg
            .iter()
            .cloned()
            .chain(conditions(&declared.attrs))
            .collect();
        let explicit = declared.attrs.iter().find_map(|attr| match &attr.meta {
            syn::Meta::NameValue(value) if attr.path().is_ident("path") => match &value.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(path),
                    ..
                }) => Some(path.value()),
                _ => None,
            },
            _ => None,
        });
        // `#[cfg_attr(unix, path = "a/unix.rs")]`: each target's file, under its condition.
        let per_target: Vec<(String, String)> = declared
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("cfg_attr"))
            .filter_map(|attr| {
                let args = attr
                    .parse_args_with(
                        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                    )
                    .ok()?;
                let mut args = args.into_iter();
                let condition = args.next()?;
                let path = args.find_map(|meta| match meta {
                    syn::Meta::NameValue(value) if value.path.is_ident("path") => {
                        match value.value {
                            syn::Expr::Lit(syn::ExprLit {
                                lit: syn::Lit::Str(path),
                                ..
                            }) => Some(path.value()),
                            _ => None,
                        }
                    }
                    _ => None,
                })?;
                Some((condition.to_token_stream().to_string(), path))
            })
            .collect();
        if declared.content.is_none() && explicit.is_none() && !per_target.is_empty() {
            for (condition, relative) in per_target {
                let file = here.join(relative);
                let dir = file
                    .parent()
                    .expect("a module file has a folder")
                    .to_owned();
                let cfg = cfg.iter().cloned().chain([condition]).collect();
                pending.push((child.clone(), file, dir, cfg));
            }
            continue;
        }
        if let Some((_, inner)) = &declared.content {
            // An inline module's children live in its folder, named by its own `#[path]` if any.
            let inner_dir = dir.join(explicit.as_deref().unwrap_or(&name));
            children(
                &child,
                inner,
                &inner_dir,
                &inner_dir,
                include_directory,
                &cfg,
                every_build,
                include_tests,
                pending,
            );
            continue;
        }
        let (file, child_dir) = match explicit {
            Some(relative) => {
                let file = here.join(relative);
                let dir = file
                    .parent()
                    .expect("a module file has a folder")
                    .to_owned();
                (file, dir)
            }
            None if dir.join(format!("{name}.rs")).is_file() => {
                (dir.join(format!("{name}.rs")), dir.join(&name))
            }
            None => (dir.join(&name).join("mod.rs"), dir.join(&name)),
        };
        pending.push((child, file, child_dir, cfg));
    }
}

/// Whether `attrs` hold a `cfg` (or `cfg_attr`): the item is compiled only in some builds.
pub fn gated(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
}
