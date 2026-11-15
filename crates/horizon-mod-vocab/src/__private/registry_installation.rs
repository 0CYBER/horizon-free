//! Opaque identities for host-owned immutable numeric registry snapshots.

/// Identify the retained mod list and, if installed, its immutable Parts epoch.
///
/// These process-local tokens are host plumbing, not a public mod API or wire
/// identity. Installed lists and Parts are retained for the process lifetime;
/// their addresses cannot be recycled while a registry refers to them. Empty
/// mod lists are equivalent regardless of the allocation that supplied them.
/// A registrar still building the next installation has no immutable Parts identity yet.
pub fn registry_installation_identity() -> (usize, Option<usize>) {
    let mods = crate::mods();
    let mods = if mods.is_empty() {
        0
    } else {
        mods.as_ptr() as usize
    };
    let parts = if crate::parts::registration_in_scope() {
        None
    } else {
        crate::parts::installed().map(|parts| parts as *const _ as usize)
    };
    (mods, parts)
}
