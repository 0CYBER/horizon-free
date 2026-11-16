//! Discover ordinary Parts modules, without joining their runtime bodies.
mod installation;
use crate::catalog::Catalog;
use std::{fs, path::Path};
use syn::{Item, Visibility};

pub(crate) const DIRECTORY: &str = "crates/horizon-mod-vocab/src/parts";
pub(crate) const OUTPUT: &str = "parts_modules.rs";

fn visibility(item: &Item) -> Option<&Visibility> {
    match item {
        Item::Const(i) => Some(&i.vis),
        Item::Enum(i) => Some(&i.vis),
        Item::Fn(i) => Some(&i.vis),
        Item::Static(i) => Some(&i.vis),
        Item::Struct(i) => Some(&i.vis),
        Item::Trait(i) => Some(&i.vis),
        Item::Type(i) => Some(&i.vis),
        Item::Use(i) => Some(&i.vis),
        _ => None,
    }
}

fn reexport(items: &[Item], module: &str) -> String {
    let mut visible = items.iter().filter_map(visibility);
    let public = visible.clone().any(|v| matches!(v, Visibility::Public(_)));
    if !public && !visible.any(|v| !matches!(v, Visibility::Inherited)) {
        return String::new();
    }
    let scope = if public { "pub" } else { "pub(crate)" };
    // Re-exports form the internal cross-kind namespace, even for a kind whose
    // helpers currently have no sibling consumer in one feature configuration.
    format!("#[allow(unused_imports)] {scope} use {module}::*;\n")
}

pub(crate) fn source(root: &Path) -> Result<String, String> {
    let dir = root.join(DIRECTORY);
    let mut modules = String::new();
    let mut fixtures = String::new();
    let mut unions = [Vec::new(), Vec::new()];
    for kind in Catalog::modules(&dir).map_err(|e| e.to_string())? {
        if kind == "mod" {
            continue;
        }
        let path = dir.join(format!("{kind}.rs"));
        let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let file = syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
        let path = path.to_string_lossy();
        modules.push_str(&format!("#[path = {path:?}] mod {kind};\n"));
        modules.push_str(&reexport(&file.items, &kind));
        for item in &file.items {
            if let Item::Trait(item) = item {
                let name = item.ident.to_string();
                for (index, suffix) in ["Registrar", "CapabilityProviders"].iter().enumerate() {
                    if name.ends_with(suffix) {
                        unions[index].push(format!("crate::parts::{name}"));
                    }
                }
            }
        }
        for item in &file.items {
            if let Item::Mod(module) = item
                && module.ident == "tests"
                && !matches!(module.vis, Visibility::Inherited)
                && let Some((_, items)) = &module.content
            {
                fixtures.push_str(&reexport(items, &format!("super::{kind}::tests")));
            }
        }
    }
    let mut template = fs::read_to_string(dir.join("unions.rs.in")).map_err(|e| e.to_string())?;
    for (owner, members) in ["Registrar", "CapabilityProviders"].iter().zip(unions) {
        let declaration = format!("pub trait {owner} {{}}");
        let bounds = if members.is_empty() {
            String::new()
        } else {
            format!(": {}", members.join(" + "))
        };
        template = template.replace(&declaration, &format!("pub trait {owner}{bounds} {{}}"));
    }
    syn::parse_file(&template).map_err(|e| e.to_string())?;
    modules.push_str(&template);
    modules.push_str(&installation::source(root)?);
    modules.push_str(&format!(
        "/// Shared Parts fixtures for engine tests.\n#[cfg(any(test, feature = \"testing\"))]\npub mod tests {{\n{fixtures}}}\n"
    ));
    Ok(modules)
}

/// Discover concrete authoring modules independently from the vocabulary's area traits.
pub(crate) fn registration_source(root: &Path) -> Result<String, String> {
    let dir = root.join("crates/horizon-mod-api/src/parts");
    if !dir.exists() {
        return Ok(String::new());
    }
    let mut modules = String::new();
    let mut fixtures = String::new();
    for kind in Catalog::modules(&dir).map_err(|e| e.to_string())? {
        if kind == "mod" {
            continue;
        }
        let path = dir.join(format!("{kind}.rs"));
        let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let file = syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
        modules.push_str(&format!(
            "#[path = {:?}] mod {kind};\n",
            path.to_string_lossy()
        ));
        modules.push_str(&reexport(&file.items, &kind));
        for item in &file.items {
            if let Item::Mod(module) = item
                && module.ident == "tests"
                && !matches!(module.vis, Visibility::Inherited)
                && let Some((_, items)) = &module.content
            {
                let mut items = items.clone();
                // The fixture include is deliberately lexical, not a child module.
                // Account for its discovered items when deciding re-export visibility.
                let fragments = dir.join(format!("{kind}_fixtures"));
                if fragments.is_dir() {
                    for name in Catalog::modules(&fragments).map_err(|e| e.to_string())? {
                        let path = fragments.join(format!("{name}.rs"));
                        let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                        let file = syn::parse_file(&source)
                            .map_err(|e| format!("{}: {e}", path.display()))?;
                        items.extend(file.items);
                    }
                }
                fixtures.push_str(&reexport(&items, &format!("super::{kind}::tests")));
            }
        }
    }
    modules.push_str(&format!(
        "#[cfg(any(test, feature = \"testing\"))] pub mod tests {{ {fixtures} }}\n"
    ));
    Ok(modules)
}

/// Export just the discovered area traits; runtime-only Parts remain private.
pub(crate) fn trait_exports(root: &Path) -> Result<String, String> {
    let file = syn::parse_file(&source(root)?).map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    for item in file.items {
        if let Item::Trait(item) = item {
            for bound in item.supertraits {
                if let syn::TypeParamBound::Trait(bound) = bound {
                    names.push(bound.path.segments.last().unwrap().ident.to_string());
                }
            }
        }
    }
    if names.is_empty() {
        return Ok(String::new());
    }
    Ok(format!("pub use crate::parts::{{{}}};\n", names.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn a_new_kind_supplies_union_methods_impls_and_fixtures_without_shared_edits() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/parts-union-discovery-{}",
            std::process::id()
        ));
        let dir = root.join(DIRECTORY);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("mod.rs"), "").unwrap();
        fs::write(
            dir.join("unions.rs.in"),
            "pub trait Registrar {} pub trait CapabilityProviders {}\n",
        )
        .unwrap();
        fs::write(
            dir.join("alpha.rs"),
            r#"
            pub trait AlphaRegistrar { fn add(&mut self); }
            impl AlphaRegistrar for super::FakeRegistrar { fn add(&mut self) { self.0 += 1; } }
            #[cfg(test)] pub mod tests {
                pub fn seeded() -> super::super::FakeRegistrar { super::super::FakeRegistrar(5) }
                #[test] fn old_kind_through_union() {
                    let mut r = seeded();
                    let u: &mut dyn super::super::Registrar = &mut r;
                    u.add(); assert_eq!(r.0, 6);
                }
            }
        "#,
        )
        .unwrap();
        let shared = r#"
            include!("exported.rs");
            mod parts {
                include!("generated.rs");
                pub struct FakeRegistrar(pub u32);
                pub struct FakeProviders;
                impl Registrar for FakeRegistrar {}
                impl CapabilityProviders for FakeProviders {}
            }
        "#;
        let wrapper = root.join("wrapper.rs");
        fs::write(&wrapper, shared).unwrap();
        let compile_and_run = || {
            fs::write(root.join("generated.rs"), source(&root).unwrap()).unwrap();
            fs::write(root.join("exported.rs"), trait_exports(&root).unwrap()).unwrap();
            let binary = root.join(format!("union-test{}", std::env::consts::EXE_SUFFIX));
            let output = std::process::Command::new("rustc")
                .args(["--edition=2024", "--test"])
                .arg(&wrapper)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = std::process::Command::new(binary).output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            String::from_utf8(output.stdout).unwrap()
        };
        assert!(compile_and_run().contains("old_kind_through_union"));
        fs::write(dir.join("beta.rs"), r#"
            pub trait BetaRegistrar { fn double(&mut self); }
            impl BetaRegistrar for super::FakeRegistrar { fn double(&mut self) { self.0 *= 2; } }
            pub trait BetaCapabilityProviders { fn ready(&self) -> bool; }
            impl BetaCapabilityProviders for super::FakeProviders { fn ready(&self) -> bool { true } }
            #[cfg(test)] pub mod tests {
                #[test] fn new_kind_through_unions() {
                    let mut r = super::super::tests::seeded();
                    let u: &mut dyn super::super::Registrar = &mut r;
                    u.double(); assert_eq!(r.0, 10);
                    crate::BetaRegistrar::double(&mut r); assert_eq!(r.0, 20);
                    let p: &dyn super::super::CapabilityProviders = &super::super::FakeProviders;
                    assert!(p.ready());
                }
            }
        "#).unwrap();
        assert!(compile_and_run().contains("new_kind_through_unions"));
        assert_eq!(fs::read_to_string(wrapper).unwrap(), shared);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_parts_kind_needs_no_shared_list() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/parts-discovery-{}",
            std::process::id()
        ));
        let dir = root.join(DIRECTORY);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("mod.rs"), "").unwrap();
        fs::write(
            dir.join("unions.rs.in"),
            "pub trait Registrar {} pub trait CapabilityProviders {}\n",
        )
        .unwrap();
        fs::write(
            dir.join("zeta.rs"),
            "pub struct Last; #[cfg(test)] mod tests { fn local_test() {} }",
        )
        .unwrap();
        let first = source(&root).unwrap();
        assert!(first.contains("mod zeta;"));
        assert!(first.contains("pub use zeta::*;"));
        assert!(!first.contains("super::zeta::tests"));
        fs::write(
            dir.join("alpha.rs"),
            "pub struct First; #[cfg(test)] pub mod tests { pub fn fixture() {} }",
        )
        .unwrap();
        let added = source(&root).unwrap();
        syn::parse_file(&added).unwrap();
        assert!(added.find("mod alpha;") < added.find("mod zeta;"));
        assert!(added.contains("pub use super::alpha::tests::*;"));
        assert!(!added.contains("struct First"));
        fs::remove_file(dir.join("alpha.rs")).unwrap();
        let removed = source(&root).unwrap();
        assert!(!removed.contains("alpha"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn discovered_fixture_items_keep_the_shared_namespace() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/registration-fixture-discovery-{}",
            std::process::id()
        ));
        let dir = root.join("crates/horizon-mod-api/src/parts");
        fs::create_dir_all(dir.join("sample_fixtures")).unwrap();
        fs::write(
            dir.join("sample.rs"),
            "pub mod tests { include!(concat!(env!(\"OUT_DIR\"), \"/sample_fixtures.rs\")); }",
        )
        .unwrap();
        assert!(
            !registration_source(&root)
                .unwrap()
                .contains("super::sample::tests")
        );
        fs::write(
            dir.join("sample_fixtures/method.rs"),
            "pub fn fixture() {}\n",
        )
        .unwrap();
        let generated = registration_source(&root).unwrap();
        syn::parse_file(&generated).unwrap();
        assert!(generated.contains("pub use super::sample::tests::*;"));
        fs::remove_dir_all(root).unwrap();
    }
}
