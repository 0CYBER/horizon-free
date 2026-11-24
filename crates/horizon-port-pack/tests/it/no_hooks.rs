//! The mod's hooks for this native test binary, which drives no guest code of its own.
//!
//! In a workspace build `horizon-mod-wasm` turns `wasm-guest` on for `mod_api`, which then declares
//! `__horizon_mod_hooks` for `#[mod_api::horizon_mod]` to define in a real mod. MSVC's linker refuses
//! the unresolved symbol even though nothing calls it, where GNU ld drops the unused reference. The
//! same stub as `horizon-mod-api`'s `tests/support/no_hooks.rs`, kept here so this crate reads no
//! other crate's files.

#[unsafe(no_mangle)]
fn __horizon_mod_hooks() -> Box<dyn mod_api::Hooks> {
    struct Empty;
    impl mod_api::Hooks for Empty {
        fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
            &[]
        }
    }
    Box::new(Empty)
}
