//! The mod's hooks for a native test binary that drives no guest code of its own.
//!
//! This crate's tests take `mod_api` with `wasm-guest`, which declares `__horizon_mod_hooks` for
//! `#[mod_api::horizon_mod]` to define in a real mod. MSVC's linker refuses the unresolved symbol
//! even though nothing calls it, where GNU ld drops the unused reference, so each test binary
//! without hooks of its own includes this one.

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
