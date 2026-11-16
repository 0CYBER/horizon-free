//! Platform-owned warning history, never part of an engine lookup table.
use std::{
    collections::HashSet,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
static BROKEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static OFF_THREAD_WARNED: AtomicBool = AtomicBool::new(false);

pub(crate) const DIAGNOSTICS: mod_vocab::dispatch_scope::Diagnostics =
    mod_vocab::dispatch_scope::Diagnostics {
        first_broken_warning: |owner| {
            BROKEN
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_or_insert_default()
                .insert(owner.to_owned())
        },
        first_off_thread_warning: || !OFF_THREAD_WARNED.swap(true, Ordering::Relaxed),
    };

#[cfg(test)]
pub(crate) fn warned_about(owner: &str) -> bool {
    BROKEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|owners| owners.contains(owner))
}
