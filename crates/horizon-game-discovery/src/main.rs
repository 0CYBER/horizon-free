//! Source-only test identities for CI. Input: PACKAGE<TAB>TARGET<TAB>crate-root<TAB>manifest-directory.
//! Output: JSONL records with package, target, path, line, name, ignored and reason.
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead},
    path::{Path, PathBuf},
};
use syn::{Attribute, Expr, Item, Lit, Meta, parse::Parser};

fn literal(attribute: &Attribute) -> Option<String> {
    let Meta::NameValue(value) = &attribute.meta else {
        return None;
    };
    let Expr::Lit(value) = &value.value else {
        return None;
    };
    let Lit::Str(value) = &value.lit else {
        return None;
    };
    Some(value.value())
}

fn parse(
    root: &Path,
    manifest: &Path,
    exports: Option<&BTreeMap<String, String>>,
    path: &Path,
    source: &str,
) -> Result<syn::File, String> {
    let mut parsed = syn::parse_file(source).map_err(|e| format!("{}: {e}", path.display()))?;
    parsed.items = expand_items(root, manifest, exports, path, parsed.items)?;
    Ok(parsed)
}

fn expand_items(
    root: &Path,
    manifest: &Path,
    exports: Option<&BTreeMap<String, String>>,
    path: &Path,
    original: Vec<Item>,
) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    for mut item in original {
        if let Item::Mod(child) = &mut item
            && let Some((_, inner)) = &mut child.content
        {
            *inner = expand_items(root, manifest, exports, path, std::mem::take(inner))?;
        }
        if let Item::Macro(included) = &mut item
            && included.mac.path.is_ident("include")
        {
            let tokens = included.mac.tokens.to_string();
            if tokens.contains(horizon_game_discovery::exports::PREFIX) {
                let sources =
                    exports.ok_or("vocabulary exports include without its source fragments")?;
                let (_, source) = sources
                    .iter()
                    .find(|(name, _)| tokens.contains(name.as_str()))
                    .ok_or_else(|| format!("unknown vocabulary export include: {tokens}"))?;
                items.extend(syn::parse_file(source).map_err(|e| e.to_string())?.items);
                continue;
            }
            let generated = if tokens.contains("game_modules.rs")
                && path.ends_with("crates/horizon-client/src/game.rs")
            {
                Some(horizon_game_discovery::declarations(
                    &root.join("crates/horizon-client/src/game"),
                    &["native_hooks", "host", "creatures"],
                )?)
            } else if tokens.contains("builtin_ingredient_test_modules.rs") {
                Some(horizon_game_discovery::folder_modules(&root.join(
                    "crates/horizon-client/src/crafting/ingredient/builtin_kinds_tests/families",
                ))?)
            } else if tokens.contains("native_hook_modules.rs") {
                Some(horizon_game_discovery::native_hook_modules(root)?)
            } else if tokens.contains("game_host_modules.rs") {
                Some(horizon_game_discovery::host_modules(root)?)
            } else if tokens.contains("lan_action_modules.rs") {
                let directory = path.parent().ok_or("LAN actions source has no parent")?;
                Some(horizon_game_discovery::lan_action_modules(
                    directory,
                    horizon_game_discovery::module_files(directory)?,
                )?)
            } else {
                None
            };
            if let Some(generated) = generated {
                items.extend(
                    syn::parse_file(&generated)
                        .map_err(|e| e.to_string())?
                        .items,
                );
                continue;
            }
            if let Some(path) = manifest_include(included.mac.tokens.clone(), manifest)? {
                let path = syn::LitStr::new(&path, included.mac.path.segments[0].ident.span());
                included.mac.tokens = quote::quote!(#path);
            }
        }
        items.push(item);
    }
    Ok(items)
}

// Evaluate only the compiler's literal concat!(env!("CARGO_MANIFEST_DIR"), "...") form.
// Other build-time includes are not arbitrary code for this source-only tool to execute.
fn manifest_include(
    tokens: proc_macro2::TokenStream,
    manifest: &Path,
) -> Result<Option<String>, String> {
    let Ok(Expr::Macro(expression)) = syn::parse2::<Expr>(tokens) else {
        return Ok(None);
    };
    if !expression.mac.path.is_ident("concat") {
        return Ok(None);
    }
    let arguments = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated
        .parse2(expression.mac.tokens)
        .map_err(|e| e.to_string())?;
    let mut path = String::new();
    for argument in arguments {
        match argument {
            Expr::Lit(syn::ExprLit {
                lit: Lit::Str(value),
                ..
            }) => {
                path.push_str(&value.value());
            }
            Expr::Macro(value) if value.mac.path.is_ident("env") => {
                let name =
                    syn::parse2::<syn::LitStr>(value.mac.tokens).map_err(|e| e.to_string())?;
                if name.value() != "CARGO_MANIFEST_DIR" {
                    return Ok(None);
                }
                path.push_str(manifest.to_str().ok_or("manifest directory is not UTF-8")?);
            }
            _ => return Ok(None),
        }
    }
    Ok(Some(path))
}

fn emit(
    package: &str,
    target: &str,
    root: &Path,
    file: &Path,
    module: &str,
    items: &[Item],
) -> Result<(), String> {
    for item in items {
        match item {
            Item::Mod(child) => {
                if let Some((_, items)) = &child.content {
                    let name = if module.is_empty() {
                        child.ident.to_string()
                    } else {
                        format!("{module}::{}", child.ident)
                    };
                    emit(package, target, root, file, &name, items)?;
                }
            }
            Item::Fn(function)
                if function
                    .attrs
                    .iter()
                    .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test")) =>
            {
                let name = if module.is_empty() {
                    function.sig.ident.to_string()
                } else {
                    format!("{module}::{}", function.sig.ident)
                };
                let attribute = function.attrs.iter().find(|a| a.path().is_ident("ignore"));
                let reason = match attribute {
                    None => None,
                    Some(attribute) if matches!(attribute.meta, Meta::Path(_)) => None,
                    Some(attribute) => Some(literal(attribute).ok_or_else(|| {
                        format!("{}: {name}: ignore reason must be a string", file.display())
                    })?),
                };
                let relative = file
                    .strip_prefix(root)
                    .map_err(|e| format!("{}: {e}", file.display()))?;
                println!(
                    "{}",
                    serde_json::json!({
                        "package": package, "target": target, "path": relative,
                        "line": function.sig.fn_token.span.start().line, "name": name,
                        "ignored": attribute.is_some(), "reason": reason,
                    })
                );
            }
            _ => {}
        }
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("usage: horizon-game-discovery ROOT < targets.tsv")?;
    let root = PathBuf::from(root)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let export_directory = root.join("crates/horizon-mod-vocab/src/exports");
    let exports = if export_directory.is_dir() {
        let (scopes, _, _) = horizon_game_discovery::exports::fragments(
            &root,
            horizon_game_discovery::module_files(&export_directory)?,
        )?;
        Some(
            scopes
                .into_iter()
                .map(|(scope, source)| {
                    (
                        format!("{}{scope}.rs", horizon_game_discovery::exports::PREFIX),
                        source,
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        )
    } else {
        None
    };
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let fields: Vec<_> = line.split('\t').collect();
        let [package, target, path, manifest] = fields.as_slice() else {
            return Err("expected PACKAGE<TAB>TARGET<TAB>crate-root<TAB>manifest-directory".into());
        };
        let manifest = root.join(manifest);
        let file = root.join(path);
        let directory = file
            .parent()
            .ok_or("crate root has no parent")?
            .to_path_buf();
        let files = mod_source::modules::walk_module_files(
            "root".into(),
            file,
            directory,
            false,
            true,
            |path, source| parse(&root, &manifest, exports.as_ref(), path, source),
        )?;
        for file in files {
            let path = file
                .path
                .canonicalize()
                .map_err(|e| format!("{}: {e}", file.path.display()))?;
            let source =
                fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let parsed =
                syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
            let module = if file.module == "root" {
                ""
            } else {
                file.module
                    .strip_prefix("root::")
                    .ok_or("module walker returned an unexpected root")?
            };
            emit(package, target, &root, &path, module, &parsed.items)?;
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
