//! Borrowed test-host capabilities without publishing native representation fields.
/// Bind a shared test backend for this borrow.
#[cfg(any(test, feature = "testing"))]
pub fn backend_ref(inner: &dyn crate::Backend) -> crate::BackendRef<'_> {
    crate::BackendRef::of(inner)
}
/// Bind an exclusive test backend for this borrow.
#[cfg(any(test, feature = "testing"))]
pub fn backend_mut_ref(inner: &mut dyn crate::BackendMut) -> crate::BackendMutRef<'_> {
    crate::BackendMutRef::of(inner)
}
