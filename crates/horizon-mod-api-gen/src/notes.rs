//! `docs/modding/src/reference/hook-notes.md` from each declaration's `notes` attribute.
//!
//! A hook or helper carries its prose on its own declaration, so two PRs that add different
//! hooks never edit the same page. The book preprocessor uses `cargo run -p xtask -- mod-api notes`
//! to render the chapter in memory; no generated Markdown is committed.
use crate::catalog::Catalog;
use std::path::Path;

/// The page's title, before the declarations' sections.
const HEADER: &str = "# Hook notes\n";

/// Renders the page from the declarations' notes, in catalog order: the general notes, then the hooks'.
pub fn render(catalog: &Catalog) -> String {
    let mut page = String::from(HEADER);
    for notes in catalog.general_notes.as_deref().into_iter().chain(
        catalog
            .hooks
            .iter()
            .filter_map(|hook| hook.notes.as_deref()),
    ) {
        page.push('\n');
        page.push_str(notes.trim_end());
        page.push('\n');
    }
    page
}

/// Reads the checkout's catalog and renders the page.
pub fn generate(root: &Path) -> Result<String, String> {
    let catalog = Catalog::load(root).map_err(|error| error.to_string())?;
    Ok(render(&catalog))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn renders_general_and_hook_notes_without_a_checked_in_page() {
        let source = r###"
            trait Notes {
                #[notes = "## General"]
                fn general();
            }
            trait Hooks {
                /// Test hook.
                #[hook(side = "server", contract = "listen")]
                #[notes = "## Hook\n\nFirst."]
                #[notes = "Second."]
                fn example();
                /// Hook with no extra prose.
                #[hook(side = "server", contract = "listen")]
                fn no_notes();
            }
        "###;
        let catalog = crate::catalog::Catalog::parse(&[("test.rs".into(), source.into())]).unwrap();
        assert_eq!(
            super::render(&catalog),
            "# Hook notes\n\n## General\n\n## Hook\n\nFirst.\n\nSecond.\n"
        );
    }

    #[test]
    fn the_checkout_renders_without_writing_a_source_page() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("docs/modding/src/reference/hook-notes.md");
        assert!(
            !path.exists(),
            "generated book pages must not be checked in"
        );
        let page = super::generate(&root).unwrap();
        assert!(page.contains("## Counterparts"));
        assert!(!path.exists(), "rendering must not write into book sources");
    }
}
