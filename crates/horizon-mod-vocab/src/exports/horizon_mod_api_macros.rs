/// Encode an owned value with the shared native and wasm Wire contract.
///
/// Fields are encoded in declaration order. Enum tags are zero-based declaration
/// indices, independent of explicit discriminants. Use `#[wire(via = Type)]` to
/// encode through `From<&Self>` and validate decoding with `TryFrom<Type>`.
/// Borrowed call arguments are not owned Wire values.
///
/// ```compile_fail
/// #[derive(mod_api::Wire)]
/// struct Borrowed<'a> { bytes: &'a [u8] }
/// ```
///
/// ```compile_fail
/// struct NoWire;
/// #[derive(mod_api::Wire)]
/// struct MissingFieldForm { value: NoWire }
/// ```
///
/// ```compile_fail
/// #[derive(mod_api::Wire)]
/// #[wire(via = String)]
/// struct MissingConversions(u32);
/// ```
pub use horizon_mod_api_macros::Wire;

/// Internal test-fixture attribute for modules with multiple hook sets; mods use `#[mod_api::horizon_mod]`.
#[doc(hidden)]
pub use horizon_mod_api_macros::__subscriptions;
/// Mark a hook implementation and derive its subscriptions.
///
/// On `impl Hooks`, also exports `pub fn hooks()` using the mod's `Default`
/// implementation. Put that impl at the crate root so the launcher finds it.
///
/// An implementation without the attribute does not compile:
/// ```compile_fail
/// use mod_api::Hooks;
/// struct Mod;
/// impl Hooks for Mod {}
/// ```
/// Rustc rejects a misspelled trait method (E0407):
/// ```compile_fail,E0407
/// use mod_api::{self, Hooks};
/// #[derive(Default)]
/// struct Mod;
/// #[mod_api::horizon_mod]
/// impl Hooks for Mod {
///     fn baby_entity_spwan(&self) {}
/// }
/// ```
/// Two mod markers in one module define the same export:
/// ```compile_fail,E0428
/// use mod_api::{self, Hooks};
/// #[derive(Default)] struct A;
/// #[derive(Default)] struct B;
/// #[mod_api::horizon_mod] impl Hooks for A {}
/// #[mod_api::horizon_mod] impl Hooks for B {}
/// ```
pub use horizon_mod_api_macros::{horizon_mod, horizon_part, if_installed};
