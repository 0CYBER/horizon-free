//! Executable synthetic mixed-output catalog fixture; no production fishing hook.
use super::*;

pub fn source(root: &Path) -> Result<String, String> {
    let catalog = Catalog::parse(&[(
        "fixture.rs".into(),
        r#"
        pub trait Hooks {
            /// Mixed-output fixture.
            #[hook(side = "server", contract = "cancel", out = "cancel")]
            fn mixed();
            /// Scalar-only fixture.
            #[hook(side = "server", contract = "cancel", out = "cancel")]
            fn scalar();
            /// Stack-only fixture.
            #[hook(side = "server", contract = "listen")]
            fn stack();
        }
    "#
        .into(),
    )])
    .map_err(|error| error.to_string())?;
    let method: syn::ItemFn = syn::parse_quote! {
        fn native_mixed(&mut self, drops: &mut Vec<ItemStack>, damage: &mut i32, cancel: &mut bool) {
            self.fire_mixed(drops, damage, cancel);
        }
    };
    let scalar: syn::ItemFn = syn::parse_quote! {
        fn native_scalar(&mut self, damage: &mut i32, cancel: &mut bool) {
            self.fire_scalar(damage, cancel);
        }
    };
    let stack: syn::ItemFn = syn::parse_quote! {
        fn native_stack(&mut self, drops: &mut Vec<ItemStack>) {
            self.fire_stack(drops);
        }
    };
    let generated = dispatch::emit(root, catalog.hooks.iter(), &[method, scalar, stack], &[])?;
    Ok(include_str!("listener_output_fixture.in")
        .replace(
            "/* DISPATCH */",
            &generated
                .dispatch
                .replace("#[derive(horizon_mod_api_macros::Wire)]", ""),
        )
        .replace("/* FIRE */", &generated.fire))
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn mixed_outputs_commit_per_listener_natively() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("fixture.rs");
        let binary = directory.path().join("fixture");
        std::fs::write(&source, super::source(&root).unwrap()).unwrap();
        let result = std::process::Command::new("rustc")
            .args(["--edition=2024", "--crate-name=mixed_output_fixture"])
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = std::process::Command::new(binary).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
