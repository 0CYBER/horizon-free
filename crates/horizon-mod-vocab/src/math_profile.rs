/// The native engine's process-lifetime selection for `sin`, `cos`, `tan`,
/// `exp`, `log`, `log10` and `pow`, read through [`crate::Host`].
///
/// This describes algorithms, not machine identity. It does not execute guest
/// math: [`crate::java_math`] remains native-only. A wasm guest must supply any
/// required platform libm, correctly rounded or fused-multiply-add implementation
/// itself, or report that it cannot reproduce the selected profile.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum MathProfile {
    /// Platform libm for all seven operations (with Java's `pow` special cases).
    LibmRounding,
    /// Correctly rounded `sin`, `cos` and `log`; platform libm for `tan`,
    /// `exp`, `log10` and `pow` (including Java's `pow` special cases).
    /// This is deliberately a mixed path, not correctly rounded math throughout.
    CorrectlyRounded,
    /// fdlibm with fused multiply-adds for `sin` and `cos`; fdlibm for `log`
    /// and `log10`; the Rust `libm` crate for `tan`, `exp` and `pow` (with
    /// Java's special cases). The latter are not promised to be StrictMath.
    Aarch64Fma,
    /// fdlibm throughout. Reserved for a future selection: no current engine
    /// cfg branch selects this variant, including non-x86/non-AArch64 targets.
    StrictMath,
}
