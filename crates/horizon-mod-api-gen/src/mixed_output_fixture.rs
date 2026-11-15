//! Opt-in catalog input for the real native-host/generated-guest boundary test.
//! This is not a gameplay hook and is absent from default API/wire generation.
use super::*;

pub(super) fn append(catalog: &mut Catalog, methods: &mut Vec<syn::ItemFn>) -> Result<(), String> {
    let fixture = Catalog::parse(&[(
        "mixed_output_fixture.rs".into(),
        r#"
        pub trait Hooks {
            /// Test-only stack-list, scalar and cancellation boundary contract.
            #[hook(side = "server", contract = "cancel", out = "cancel")]
            fn mixed_output_fixture();
        }
        "#
        .into(),
    )])
    .map_err(|error| error.to_string())?;
    catalog.hooks.extend(fixture.hooks);
    methods.push(syn::parse_quote! {
        fn native_mixed_output_fixture(
            &mut self,
            mode: u8,
            drops: &mut Vec<ItemStack>,
            damage: &mut i32,
            cancel: &mut bool,
        ) {
            self.fire_mixed_output_fixture(mode, drops, damage, cancel);
        }
    });
    Ok(())
}
