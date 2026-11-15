//! The provider limit a position's capability lookup keeps its place within.
/// The most providers one capability may have for one block or one block entity type, which
/// the registrar refuses past.
pub fn max_providers_at() -> usize {
    crate::parts::MAX_PROVIDERS_AT
}
