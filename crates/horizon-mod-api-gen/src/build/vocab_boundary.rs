//! Validate generated vocabulary at the single build-output boundary.
use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(super) mod modules;

pub(super) type Inventory = BTreeMap<String, BTreeMap<String, String>>;

pub(super) fn inventory(root: &Path) -> Result<Inventory, String> {
    let path = root.join("tools/layering/mod_vocab_state.json");
    serde_json::from_str(
        &std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
fn validate(name: &str, code: &str, inventory: &Inventory) -> Result<(), String> {
    validate_with_modules(name, code, inventory, &BTreeSet::new())
}

pub(super) fn validate_with_modules(
    name: &str,
    code: &str,
    inventory: &Inventory,
    sources: &BTreeSet<std::path::PathBuf>,
) -> Result<(), String> {
    let prefix = format!("@generated/{name}::");
    let allowed: BTreeMap<_, _> = inventory
        .iter()
        .filter_map(|(key, entry)| {
            key.strip_prefix(&prefix)
                .map(|key| (key.to_owned(), entry.get("type")))
        })
        .collect();
    let parsed = syn::parse_file(code).map_err(|e| e.to_string())?;
    modules::validate(&parsed, sources)?;
    let mut seen = BTreeSet::new();
    for item in parsed.items {
        if let syn::Item::Static(item) = item {
            let id = item.ident.to_string();
            let actual: String = item
                .ty
                .to_token_stream()
                .to_string()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            if allowed.get(&id).copied().flatten().map(String::as_str) != Some(actual.as_str()) {
                return Err(format!(
                    "unlisted or changed generated static {id}: {actual}"
                ));
            }
            seen.insert(id);
        }
    }
    if allowed.keys().any(|name| !seen.contains(name)) {
        return Err(format!("stale generated state inventory for {name}"));
    }
    fn tokens(stream: TokenStream, top: bool, allowed: &BTreeSet<String>) -> Result<(), String> {
        let mut apostrophe = false;
        let mut stream = stream.into_iter().peekable();
        while let Some(token) = stream.next() {
            match token {
                TokenTree::Ident(name) => {
                    let name = name.to_string();
                    if name == "static" && !apostrophe {
                        let known = top
                            && matches!(stream.peek(), Some(TokenTree::Ident(id)) if allowed.contains(&id.to_string()));
                        if !known {
                            return Err("unlisted generated static or state-declaring macro".into());
                        }
                    }
                    if matches!(name.as_str(), "unsafe" | "unsafe_code" | "include") {
                        return Err(format!(
                            "generated vocabulary contains forbidden token {name}"
                        ));
                    }
                    apostrophe = false;
                }
                TokenTree::Group(group) => {
                    tokens(group.stream(), false, allowed)?;
                    apostrophe = false;
                }
                TokenTree::Punct(punct) => apostrophe = punct.as_char() == '\'',
                TokenTree::Literal(_) => apostrophe = false,
            }
        }
        Ok(())
    }
    tokens(
        code.parse::<TokenStream>().map_err(|e| e.to_string())?,
        true,
        &seen,
    )
}

#[cfg(test)]
mod tests {
    use super::{Inventory, validate};
    #[test]
    fn generated_state_including_macro_metavariables_is_refused() {
        for mutant in [
            "static EXTRA: AtomicBool = AtomicBool::new(false);",
            "macro_rules! slot { ($n:ident) => { static $n: AtomicBool = AtomicBool::new(false); }; }",
            "thread_local! { static EXTRA: Cell<bool> = Cell::new(false); }",
        ] {
            assert!(
                validate("mutant.rs", mutant, &Inventory::new()).is_err(),
                "{mutant}"
            );
        }
        assert!(
            validate(
                "ok.rs",
                "fn lookup() -> &'static str { \"static unsafe\" }",
                &Inventory::new()
            )
            .is_ok()
        );
    }
    #[test]
    fn generated_unsafe_suppressions_and_unchecked_includes_are_refused() {
        for mutant in [
            "unsafe fn unchecked() {}",
            "#[allow(unsafe_code)] fn quiet() {}",
            "#[expect(unsafe_code)] fn quiet() {}",
            "include!(\"unreviewed.rs\");",
        ] {
            assert!(
                validate("mutant.rs", mutant, &Inventory::new()).is_err(),
                "{mutant}"
            );
        }
    }
    #[test]
    fn inventoried_generated_cache_has_exact_type_and_no_nested_state() {
        let inventory = [(
            "@generated/wire.rs::LAYOUT".into(),
            [("type".into(), "LazyLock<u64>".into())].into(),
        )]
        .into();
        assert!(
            validate(
                "wire.rs",
                "static LAYOUT: LazyLock<u64> = make();",
                &inventory
            )
            .is_ok()
        );
        for mutant in [
            "static LAYOUT: LazyLock<bool> = make();",
            "static LAYOUT: LazyLock<u64> = { static HIDDEN: u8 = 0; make() };",
            "const LAYOUT: u64 = 0;",
        ] {
            assert!(validate("wire.rs", mutant, &inventory).is_err(), "{mutant}");
        }
    }
}
