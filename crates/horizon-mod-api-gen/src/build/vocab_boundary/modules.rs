//! Out-of-line generated modules may only refer to source scanned by the gate.
use quote::ToTokens;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use syn::visit::Visit;

pub(in crate::build) fn sources(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    fn walk(directory: &Path, files: &mut BTreeSet<PathBuf>) -> Result<(), String> {
        for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                walk(&path, files)?;
            } else if path.extension().is_some_and(|extension| extension == "rs") && path.is_file()
            {
                files.insert(path.canonicalize().map_err(|e| e.to_string())?);
            }
        }
        Ok(())
    }
    let mut files = BTreeSet::new();
    walk(&root.join("crates/horizon-mod-vocab/src"), &mut files)?;
    Ok(files)
}

pub(super) fn validate(file: &syn::File, sources: &BTreeSet<PathBuf>) -> Result<(), String> {
    struct Modules<'a> {
        sources: &'a BTreeSet<PathBuf>,
        errors: Vec<String>,
    }
    impl<'ast> Visit<'ast> for Modules<'_> {
        fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
            if module.attrs.iter().any(|attr| {
                attr.path().is_ident("cfg_attr")
                    && attr
                        .meta
                        .to_token_stream()
                        .to_string()
                        .split_whitespace()
                        .any(|token| token == "path")
            }) {
                self.errors
                    .push("conditional generated module paths are not supported".into());
            }
            if module.content.is_none() {
                let path = module.attrs.iter().find_map(|attr| {
                    if !attr.path().is_ident("path") {
                        return None;
                    }
                    if let syn::Meta::NameValue(value) = &attr.meta
                        && let syn::Expr::Lit(value) = &value.value
                        && let syn::Lit::Str(path) = &value.lit
                    {
                        return Some(path.value());
                    }
                    None
                });
                let accepted = path
                    .as_ref()
                    .and_then(|path| {
                        Path::new(path)
                            .is_absolute()
                            .then(|| Path::new(path).canonicalize().ok())
                            .flatten()
                    })
                    .is_some_and(|path| self.sources.contains(&path));
                if !accepted {
                    self.errors.push(format!(
                        "generated module {} targets source outside the scanned vocabulary set",
                        module.ident
                    ));
                }
            }
            syn::visit::visit_item_mod(self, module);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            fn declares_module(tokens: proc_macro2::TokenStream) -> bool {
                tokens.into_iter().any(|token| match token {
                    proc_macro2::TokenTree::Ident(name) => name == "mod",
                    proc_macro2::TokenTree::Group(group) => declares_module(group.stream()),
                    _ => false,
                })
            }
            if declares_module(node.tokens.clone()) {
                self.errors
                    .push("generated macros cannot declare unchecked modules".into());
            }
            syn::visit::visit_macro(self, node);
        }
    }
    let mut modules = Modules {
        sources,
        errors: Vec::new(),
    };
    modules.visit_file(file);
    if modules.errors.is_empty() {
        Ok(())
    } else {
        Err(modules.errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn out_of_set_module_is_refused_but_scanned_source_is_accepted() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let sources = super::sources(&root).unwrap();
        let source = root
            .join("crates/horizon-mod-vocab/src/dispatch_scope.rs")
            .canonicalize()
            .unwrap();
        let safe = syn::parse_file(&format!("#[path = {source:?}] mod socket;")).unwrap();
        assert!(super::validate(&safe, &sources).is_ok());
        let outside = root
            .join("crates/horizon-mod-api/src/lib.rs")
            .canonicalize()
            .unwrap();
        let escaped = syn::parse_file(&format!("#[path = {outside:?}] mod hidden;")).unwrap();
        assert!(super::validate(&escaped, &sources).is_err());
        for mutant in [
            "#[path = \"/outside/hidden.rs\"] mod hidden;",
            "mod hidden;",
            "macro_rules! hidden { () => { #[path = \"/outside/hidden.rs\"] mod hidden; }; }",
        ] {
            assert!(super::validate(&syn::parse_file(mutant).unwrap(), &sources).is_err());
        }
    }
    use std::path::Path;
}
