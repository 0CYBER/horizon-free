//! Run the real entry bodies against a controlled living-knockback hook boundary.
//! No point instrumentation or game resources: only world/context services are doubles.
use quote::quote;
use std::{fs, path::Path, process::Command};

#[test]
#[ignore = "daily"]
fn retained_knockback_entries_apply_hook_edits_cancel_and_delegate_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut modules = proc_macro2::TokenStream::new();
    for (module, path) in [
        ("entries", "mobs/knockback.rs"),
        ("player_entries", "game/mobs/creatures/knockback.rs"),
    ] {
        let source = fs::read_to_string(root.join("crates/horizon-client/src").join(path)).unwrap();
        let mut file = syn::parse_file(&source).unwrap();
        // Let the fixture call both entry forms; leave signatures and bodies intact.
        for item in &mut file.items {
            if let syn::Item::Impl(owner) = item {
                for item in &mut owner.items {
                    if let syn::ImplItem::Fn(function) = item {
                        function.vis = syn::parse_quote!(pub(crate));
                        // This dependency-free fixture omits the no-op staging marker.
                        function.attrs.retain(|attr| {
                            !attr
                                .path()
                                .segments
                                .last()
                                .is_some_and(|s| s.ident == "replaceable")
                        });
                    }
                }
            }
        }
        let module = syn::Ident::new(module, proc_macro2::Span::call_site());
        modules.extend(quote!(mod #module { #file }));
    }
    let directory = tempfile::tempdir_in(root.join("target")).unwrap();
    let source = directory.path().join("main.rs");
    let binary = directory.path().join("knockback");
    fs::write(
        &source,
        format!(
            "{}\n{}\n{}",
            include_str!("knockback_fixture/preamble.rs.txt"),
            modules,
            include_str!("knockback_fixture/main.rs.txt")
        ),
    )
    .unwrap();
    let output = Command::new("rustc")
        .args(["--edition=2024", "--crate-name=knockback_fixture"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
