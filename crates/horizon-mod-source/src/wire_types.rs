//! Discovery of concrete, publicly reachable mod wire types.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use syn::{Item, Visibility};

/// Each module's public names: the name, the path it leads to, and the conditional `pub use` that
/// declares it, if any.
type Exports = BTreeMap<String, Vec<(String, String, Option<String>)>>;

/// What one walk of the crate's modules found.
#[derive(Default)]
struct Scan {
    exports: Exports,
    /// Wire types by definition path, each with its own `#[cfg]`/`#[cfg_attr]` if it has one.
    types: BTreeMap<String, Option<String>>,
    /// Modules declared under `#[cfg]` or `#[cfg_attr]`, other than test modules.
    conditional: BTreeMap<String, String>,
    /// `#[cfg(test)]` modules: not part of the build a dependent links.
    tests: BTreeSet<String>,
}

/// Lists concrete wire types by a public path from the crate root, following re-exports.
/// A Wire type a dependent can reach only through a conditional declaration (its own `#[cfg]` or
/// `#[cfg_attr]`, an enclosing module's, or a conditional `pub use`) is refused, naming it: one
/// layout must hold for every target. Test modules and unreachable declarations are not considered.
/// Uninstantiated generic types are excluded. Uses the module walk the generator uses too, including
/// explicit module paths.
pub fn public_types(krate: &str, lib: &Path) -> Result<Vec<String>, String> {
    let dir = lib.parent().ok_or("library has no source directory")?;
    let files = crate::modules::walk_module_files(
        krate.into(),
        lib.into(),
        dir.into(),
        true,
        false,
        |path, source| syn::parse_file(source).map_err(|e| format!("{}: {e}", path.display())),
    )?;
    let mut parsed = Vec::new();
    for file in files {
        let source = fs::read_to_string(file.path).map_err(|e| e.to_string())?;
        parsed.push((
            file.module,
            syn::parse_file(&source).map_err(|e| e.to_string())?,
        ));
    }
    let public = reachable(
        krate,
        parsed
            .iter()
            .map(|(module, file)| (module.as_str(), file.items.as_slice())),
    )?;
    Ok(public.into_keys().collect())
}

/// The crate's publicly reachable wire types, each public path with the path it is defined at,
/// from its parsed module files; refused as [`public_types`] refuses them.
pub(crate) fn reachable<'a>(
    krate: &str,
    files: impl IntoIterator<Item = (&'a str, &'a [Item])>,
) -> Result<BTreeMap<String, String>, String> {
    let mut scan = Scan::default();
    for (module, items) in files {
        self::items(krate, module, items, &mut scan);
    }
    let mut found = BTreeMap::new();
    visit(krate, krate, None, &scan, &mut Vec::new(), &mut found)?;
    Ok(found)
}

fn conditional(attrs: &[syn::Attribute]) -> Option<String> {
    attrs
        .iter()
        .find(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
        .map(|a| quote::ToTokens::to_token_stream(a).to_string())
}

fn under(path: &str, module: &str) -> bool {
    path.strip_prefix(module)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
}

fn visit(
    module: &str,
    public: &str,
    via: Option<&str>,
    scan: &Scan,
    visiting: &mut Vec<String>,
    found: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    if visiting.iter().any(|m| m == module) {
        return Ok(());
    }
    visiting.push(module.into());
    for (name, target, condition) in scan.exports.get(module).into_iter().flatten() {
        if scan.tests.iter().any(|test| under(target, test)) {
            continue;
        }
        let via = condition.as_deref().or(via);
        let path = if name == "*" {
            public.to_owned()
        } else {
            format!("{public}::{name}")
        };
        if let Some(own) = scan.types.get(target) {
            let reason = own
                .as_deref()
                .map(|attr| format!("{target} carries {attr}"))
                .or_else(|| {
                    scan.conditional
                        .iter()
                        .find(|(module, _)| under(target, module))
                        .map(|(module, attr)| format!("module {module} carries {attr}"))
                });
            if let Some(reason) = reason.or_else(|| via.map(str::to_owned)) {
                return Err(format!(
                    "layouts: Wire type {path} is declared conditionally ({reason}); a Wire type a dependent can name must be unconditional"
                ));
            }
            found.insert(path, target.clone());
        } else {
            visit(target, &path, via, scan, visiting, found)?;
        }
    }
    visiting.pop();
    Ok(())
}

fn uses(
    krate: &str,
    module: &str,
    tree: &syn::UseTree,
    prefix: &str,
    out: &mut Vec<(String, String)>,
) {
    match tree {
        syn::UseTree::Path(p) => uses(
            krate,
            module,
            &p.tree,
            &format!("{prefix}{}::", p.ident),
            out,
        ),
        syn::UseTree::Group(g) => {
            for tree in &g.items {
                uses(krate, module, tree, prefix, out);
            }
        }
        other => {
            let (name, suffix) = match other {
                syn::UseTree::Name(n) => (n.ident.to_string(), n.ident.to_string()),
                syn::UseTree::Rename(n) => (n.rename.to_string(), n.ident.to_string()),
                syn::UseTree::Glob(_) => ("*".into(), String::new()),
                _ => return,
            };
            let raw = format!("{prefix}{suffix}");
            let mut parts: Vec<_> = module.split("::").collect();
            let mut path = raw.trim_end_matches("::");
            if let Some(rest) = path.strip_prefix("crate::") {
                parts = vec![krate];
                path = rest;
            } else if let Some(rest) = path.strip_prefix("self::") {
                path = rest;
            }
            while let Some(rest) = path.strip_prefix("super::") {
                parts.pop();
                path = rest;
            }
            out.push((name, format!("{}::{path}", parts.join("::"))));
        }
    }
}

fn items(krate: &str, module: &str, items: &[Item], scan: &mut Scan) {
    let manual: BTreeSet<_> = items
        .iter()
        .filter_map(|item| {
            let Item::Impl(i) = item else { return None };
            let (_, path, _) = i.trait_.as_ref()?;
            if path.segments.last()?.ident != "Wire" || !i.generics.params.is_empty() {
                return None;
            }
            let syn::Type::Path(p) = &*i.self_ty else {
                return None;
            };
            (p.path.segments.len() == 1).then(|| p.path.segments[0].ident.to_string())
        })
        .collect();
    let mentions_wire = |a: &syn::Attribute| {
        quote::ToTokens::to_token_stream(a)
            .into_iter()
            .any(|token| {
                matches!(token, proc_macro2::TokenTree::Group(_))
                    && token.to_string().contains("Wire")
            })
    };
    for item in items {
        let (ident, vis, attrs, generics) = match item {
            Item::Struct(i) => (&i.ident, &i.vis, &i.attrs, &i.generics),
            Item::Enum(i) => (&i.ident, &i.vis, &i.attrs, &i.generics),
            Item::Use(i) if matches!(i.vis, Visibility::Public(_)) => {
                let mut found = Vec::new();
                uses(krate, module, &i.tree, "", &mut found);
                let condition = conditional(&i.attrs);
                scan.exports
                    .entry(module.into())
                    .or_default()
                    .extend(found.into_iter().map(|(name, target)| {
                        let condition = condition
                            .as_ref()
                            .map(|attr| format!("the re-export of {target} carries {attr}"));
                        (name, target, condition)
                    }));
                continue;
            }
            Item::Mod(i) => {
                let child = format!("{module}::{}", i.ident);
                let test = i.attrs.iter().any(|a| {
                    a.path().is_ident("cfg")
                        && quote::ToTokens::to_token_stream(&a.meta).to_string() == "cfg (test)"
                });
                if test {
                    scan.tests.insert(child);
                    continue;
                }
                if let Some(attr) = conditional(&i.attrs) {
                    scan.conditional.insert(child.clone(), attr);
                }
                if matches!(i.vis, Visibility::Public(_)) {
                    scan.exports.entry(module.into()).or_default().push((
                        i.ident.to_string(),
                        child.clone(),
                        None,
                    ));
                }
                if let Some((_, content)) = &i.content {
                    self::items(krate, &child, content, scan);
                }
                continue;
            }
            _ => continue,
        };
        if !matches!(vis, Visibility::Public(_)) || !generics.params.is_empty() {
            continue;
        }
        let derived = attrs
            .iter()
            .filter(|a| a.path().is_ident("derive"))
            .any(|a| {
                a.parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
                .is_ok_and(|paths| {
                    paths
                        .iter()
                        .any(|p| p.segments.last().is_some_and(|s| s.ident == "Wire"))
                })
            });
        let conditionally_derived = attrs
            .iter()
            .any(|a| a.path().is_ident("cfg_attr") && mentions_wire(a));
        if derived || conditionally_derived || manual.contains(&ident.to_string()) {
            let path = format!("{module}::{ident}");
            scan.types.insert(path.clone(), conditional(attrs));
            scan.exports
                .entry(module.into())
                .or_default()
                .push((ident.to_string(), path, None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_module_paths_and_manual_codecs_are_discovered() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/wire-discovery-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("lib.rs"), "#[derive(mod_api::Wire)] pub struct Root; #[derive(Wire)] struct Private; pub mod visible; mod hidden; pub use hidden::Hidden as Reexported; pub mod inline { #[derive(Wire)] pub enum Value { A } } pub struct Manual; impl wire::Wire for Manual {} #[cfg(test)] mod tests; ").unwrap();
        fs::write(root.join("visible.rs"), "#[derive(Wire)] pub struct Child; #[cfg(unix)] #[derive(Wire)] pub struct Conditional;").unwrap();
        fs::write(root.join("hidden.rs"), "#[derive(Wire)] pub struct Hidden;").unwrap();
        let refused = |visible: &str| {
            fs::write(root.join("visible.rs"), visible).unwrap();
            public_types("library", &root.join("lib.rs"))
        };
        assert!(
            refused("#[derive(Wire)] pub struct Child; #[cfg(unix)] #[derive(Wire)] pub struct Conditional;")
                .unwrap_err()
                .contains("library::visible::Conditional")
        );
        for (declaration, named) in [
            (
                "#[cfg_attr(unix, derive(Wire))] pub struct Conditional;",
                "library::visible::Conditional",
            ),
            (
                "#[cfg(unix)] pub mod gated { #[derive(Wire)] pub struct T; }",
                "library::visible::gated::T",
            ),
            (
                "#[cfg(unix)] mod gated { #[derive(Wire)] pub struct T; } pub use gated::T;",
                "library::visible::T",
            ),
            (
                "#[cfg_attr(unix, cfg(unix))] pub use crate::Root as Again;",
                "re-export of library::Root",
            ),
        ] {
            let error = refused(declaration).unwrap_err();
            assert!(error.contains(named), "{declaration}: {error}");
        }
        // Conditional declarations outside the reachable Wire surface are not refused.
        for declaration in [
            "#[derive(Wire)] pub struct Child; #[cfg(unix)] pub struct Ordinary;",
            "#[derive(Wire)] pub struct Child; mod hidden { #[cfg(unix)] #[derive(Wire)] pub struct Ordinary; }",
            "#[derive(Wire)] pub struct Child; #[cfg(test)] mod tests { #[cfg(unix)] #[derive(Wire)] pub struct Ordinary; }",
            "#[derive(Wire)] pub struct Child; #[cfg(test)] pub mod tests { #[derive(Wire)] pub struct Ordinary; }",
            "#[derive(Wire)] pub struct Child; #[cfg(unix)] pub mod plain { pub struct Ordinary; }",
        ] {
            assert_eq!(
                refused(declaration).unwrap(),
                [
                    "library::Manual",
                    "library::Reexported",
                    "library::Root",
                    "library::inline::Value",
                    "library::visible::Child"
                ],
                "{declaration}"
            );
        }
        fs::write(root.join("visible.rs"), "#[derive(Wire)] pub struct Child;").unwrap();
        assert_eq!(
            public_types("library", &root.join("lib.rs")).unwrap(),
            vec![
                "library::Manual",
                "library::Reexported",
                "library::Root",
                "library::inline::Value",
                "library::visible::Child"
            ]
        );
    }
}
