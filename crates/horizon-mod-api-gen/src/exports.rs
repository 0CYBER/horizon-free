//! Per-area API surfaces, assembled at their original lexical scopes.
use crate::catalog::Catalog;
use quote::ToTokens;
use std::{collections::BTreeMap, path::Path};
use syn::Item;

pub(crate) const DIRECTORY: &str = "crates/horizon-mod-vocab/src/exports";
use horizon_game_discovery::exports::PREFIX;

pub(crate) fn sources(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let (mut out, has_parts, has_host) = horizon_game_discovery::exports::fragments(
        root,
        Catalog::modules(&root.join(DIRECTORY)).map_err(|e| e.to_string())?,
    )?;
    if has_host {
        // Re-export the generated macro aliases, not the include!-expanded
        // macro_export implementations, at their existing crate-root paths.
        for item in syn::parse_file(&super::host::areas_source(root)?)
            .map_err(|e| e.to_string())?
            .items
        {
            if let Item::Use(item) = item
                && let syn::UseTree::Rename(rename) = item.tree
            {
                let name = rename.rename;
                out.get_mut("root")
                    .unwrap()
                    .push_str(&quote::quote!(#[doc(hidden)] pub use host::#name;).to_string());
            }
        }
    }
    if has_parts {
        out.get_mut("root")
            .unwrap()
            .push_str(&super::parts::trait_exports(root)?);
    }
    Ok(out
        .into_iter()
        .map(|(scope, code)| (format!("{PREFIX}{scope}.rs"), code))
        .collect())
}

/// Expand the same generated surfaces for source consumers (Host validation and wasm resolution).
pub(crate) fn expand(root: &Path, source: &str) -> Result<String, String> {
    fn items(input: Vec<Item>, sources: &BTreeMap<String, String>) -> Result<Vec<Item>, String> {
        let mut out = Vec::new();
        for mut item in input {
            if let Item::Macro(mac) = &item
                && mac.mac.path.is_ident("include")
                && let Some((_, code)) = sources
                    .iter()
                    .find(|(name, _)| mac.mac.tokens.to_string().contains(name.as_str()))
            {
                out.extend(syn::parse_file(code).map_err(|e| e.to_string())?.items);
                continue;
            }
            if let Item::Mod(module) = &mut item
                && let Some((_, content)) = &mut module.content
            {
                *content = items(std::mem::take(content), sources)?;
            }
            out.push(item);
        }
        Ok(out)
    }
    let parts = source.contains(super::parts::OUTPUT);
    if !source.contains(PREFIX) && !parts {
        return Ok(source.to_owned());
    }
    let mut file = syn::parse_file(source).map_err(|e| e.to_string())?;
    let mut generated = sources(root)?;
    if parts {
        generated.insert(super::parts::OUTPUT.to_owned(), super::parts::source(root)?);
    }
    file.items = items(file.items, &generated)?;
    Ok(file.to_token_stream().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn export_modules_support_file_and_directory_roots() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/export-module-roots-{}",
            std::process::id()
        ));
        let api = root.join("crates/horizon-mod-vocab/src");
        fs::create_dir_all(api.join("exports")).unwrap();
        fs::create_dir_all(api.join("parts")).unwrap();
        fs::write(
            api.join("parts/unions.rs.in"),
            "pub trait Registrar {} pub trait CapabilityProviders {}\n",
        )
        .unwrap();
        fs::write(api.join("exports/parts.rs"), "mod parts;").unwrap();
        for expected in [api.join("parts/mod.rs"), api.join("parts.rs")] {
            fs::write(&expected, "").unwrap();
            let generated = sources(&root).unwrap();
            let file = syn::parse_file(&generated["mod_api_exports_root.rs"]).unwrap();
            let Item::Mod(module) = &file.items[0] else {
                panic!("export lost its module")
            };
            let syn::Meta::NameValue(path) = &module.attrs[0].meta else {
                panic!("module has no path")
            };
            let syn::Expr::Lit(value) = &path.value else {
                panic!("module path is not a literal")
            };
            let syn::Lit::Str(value) = &value.lit else {
                panic!("module path is not a string")
            };
            assert_eq!(Path::new(&value.value()), expected);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_area_is_discovered_without_a_shared_list() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/export-discovery-{}",
            std::process::id()
        ));
        let dir = root.join(DIRECTORY);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("zeta.rs"), "pub use crate::zeta::Last;").unwrap();
        let first = sources(&root).unwrap();
        assert!(first["mod_api_exports_root.rs"].contains("Last"));
        fs::write(dir.join("alpha.rs"), "pub use crate::alpha::First; mod __private { mod built { pub use crate::alpha::Copy; } }").unwrap();
        let added = sources(&root).unwrap();
        assert!(
            added["mod_api_exports_root.rs"].find("First")
                < added["mod_api_exports_root.rs"].find("Last")
        );
        assert!(added["mod_api_exports_private_built.rs"].contains("Copy"));
        let expanded = expand(&root, "pub mod built { include!(concat!(\n env!(\"OUT_DIR\"),\n \"/mod_api_exports_private_built.rs\"\n)); }").unwrap();
        let parsed = syn::parse_file(&expanded).unwrap();
        let Item::Mod(module) = &parsed.items[0] else {
            panic!("built scope lost")
        };
        assert!(matches!(
            &module.content.as_ref().unwrap().1[0],
            Item::Use(_)
        ));
        fs::write(dir.join("invalid.rs"), "pub struct Leaked;").unwrap();
        assert!(sources(&root).unwrap_err().contains("only uses"));
        fs::remove_dir_all(root).unwrap();
    }

    /// Every `parts/<family>.rs` that declares a public name beyond its area traits is exported by
    /// `exports/parts_<family>.rs`, or says `// exports: none` on its first line.
    #[test]
    fn every_parts_family_with_public_names_has_an_export_file() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let vocab = root.join("crates/horizon-mod-vocab/src");
        let mut missing = Vec::new();
        for family in Catalog::modules(&vocab.join("parts")).unwrap() {
            let path = vocab.join(format!("parts/{family}.rs"));
            let source = fs::read_to_string(&path).unwrap();
            let file = syn::parse_file(&source).unwrap();
            let public = |vis: &syn::Visibility| matches!(vis, syn::Visibility::Public(_));
            let declares = file.items.iter().any(|item| match item {
                Item::Trait(i) => {
                    public(&i.vis)
                        && !["Registrar", "CapabilityProviders"]
                            .iter()
                            .any(|suffix| i.ident.to_string().ends_with(suffix))
                }
                Item::Fn(i) => public(&i.vis),
                Item::Struct(i) => public(&i.vis),
                Item::Enum(i) => public(&i.vis),
                Item::Const(i) => public(&i.vis),
                Item::Static(i) => public(&i.vis),
                Item::Type(i) => public(&i.vis),
                _ => false,
            });
            let exported = vocab.join(format!("exports/parts_{family}.rs")).is_file();
            let waived = source.starts_with("// exports: none");
            if declares && !exported && !waived {
                missing.push(family);
            }
        }
        assert!(
            missing.is_empty(),
            "parts families declare public names but have no exports/parts_<family>.rs \
             (add one, or start the family file with `// exports: none`): {missing:?}"
        );
    }
}
