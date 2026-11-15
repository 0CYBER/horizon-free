//! Cold authoring hooks and the scoped key-membership boundary.
use super::*;

// no wire: this selector is native registration plumbing, never a mod payload.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
#[derive(Clone, Copy)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
pub enum RegistrationKeyKind {
    Block,
    Item,
    Fluid,
    Effect,
    BlockEntityType,
    EntityType,
    GameEvent,
    Particle,
    GameRule,
}

// no wire: process-local callbacks and an API-owned activity counter.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
pub struct RegistrationAccess {
    pub active: &'static std::sync::atomic::AtomicUsize,
    pub contains: fn(RegistrationKeyKind, u32) -> Option<bool>,
    pub name: fn(RegistrationKeyKind, u32) -> Option<String>,
    pub in_scope: fn() -> bool,
    pub install: fn(
        &[ModEntry],
        &crate::__private::RuleSchema<'_>,
        fn(&crate::DimensionSettings) -> bool,
    ) -> Result<(), Vec<String>>,
}

#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn registration_membership(kind: RegistrationKeyKind, slot: u32) -> Option<bool> {
    #[cfg(test)]
    if let Some(result) =
        with_registering_keys(|keys| keys.map(|keys| names(keys, kind).id(slot).is_some()))
    {
        return Some(result);
    }
    let access = &crate::dispatch_scope::backend()?.registration;
    // Production registration finishes before dispatch. Do not call across the
    // boundary on ordinary installed-key reads.
    (access.active.load(Ordering::Relaxed) != 0)
        .then(|| (access.contains)(kind, slot))
        .flatten()
}

pub(crate) fn registration_name(kind: RegistrationKeyKind, slot: u32) -> Option<String> {
    #[cfg(test)]
    if let Some(result) =
        with_registering_keys(|keys| keys.map(|keys| names(keys, kind).id(slot).map(str::to_owned)))
    {
        return result;
    }
    let access = &crate::dispatch_scope::backend()?.registration;
    (access.active.load(Ordering::Relaxed) != 0)
        .then(|| (access.name)(kind, slot))
        .flatten()
}

pub(crate) fn registration_in_scope() -> bool {
    #[cfg(test)]
    if REGISTERING_KEYS.with_borrow(Option::is_some) {
        return true;
    }
    crate::dispatch_scope::backend().is_some_and(|backend| {
        let access = &backend.registration;
        access.active.load(Ordering::Relaxed) != 0 && (access.in_scope)()
    })
}

/// Ask the installed authoring implementation to validate and freeze declarations.
pub fn install_parts(
    mods: &[ModEntry],
    schema: &crate::__private::RuleSchema<'_>,
    dimension_shape: fn(&crate::DimensionSettings) -> bool,
) -> Result<(), Vec<String>> {
    if let Some(backend) = crate::dispatch_scope::backend() {
        (backend.registration.install)(mods, schema, dimension_shape)
    } else if mods.is_empty() {
        install_input(super::installation::InstalledInput::default());
        Ok(())
    } else {
        Err(vec![
            "mod registration requires the mod API authoring runtime".into(),
        ])
    }
}

/// Consume owned declarations and derive the lookup indexes here, never from caller bits.
pub fn install_input(input: super::installation::InstalledInput) {
    let mut installed: Installed = input.into();
    installed.keys.index_hooks();
    installed.hook_bits = installed.keys.hooks.iter().map(Option::is_some).collect();
    installed.tag_hook_bits = vec![false; HOOK_COUNT];
    for (_, part) in &installed.tags.blocks {
        for hook in part.part.block_subscribed().iter().flatten() {
            installed.hook_bits[*hook as usize] = true;
            installed.tag_hook_bits[*hook as usize] = true;
        }
    }
    for (_, part) in &installed.tags.items {
        for hook in part.part.item_subscribed().iter().flatten() {
            installed.hook_bits[*hook as usize] = true;
            installed.tag_hook_bits[*hook as usize] = true;
        }
    }
    #[cfg(any(test, feature = "testing"))]
    TEST_PARTS.set(Some(Box::leak(Box::new(installed))));
    #[cfg(not(any(test, feature = "testing")))]
    if INSTALLED.set(installed).is_err() {
        log::warn!("Mod parts already installed; keeping the first table");
    }
}

// Unit Wire fixtures inject their own lookup rows, not an authoring implementation.
#[cfg(test)]
thread_local! {
    pub(crate) static REGISTERING_KEYS: RefCell<Option<Rc<RefCell<horizon_mod_runtime::registry_keys::Requests>>>> = const { RefCell::new(None) };
}
#[cfg(test)]
pub(crate) fn with_registering_keys<T>(
    read: impl FnOnce(Option<&horizon_mod_runtime::registry_keys::Requests>) -> T,
) -> T {
    REGISTERING_KEYS.with(|keys| match keys.borrow().as_ref() {
        Some(keys) => read(Some(&keys.borrow())),
        None => read(None),
    })
}
#[cfg(test)]
fn names(
    keys: &horizon_mod_runtime::registry_keys::Requests,
    kind: RegistrationKeyKind,
) -> &horizon_mod_runtime::registry_keys::Names {
    match kind {
        RegistrationKeyKind::Block => &keys.blocks,
        RegistrationKeyKind::Item => &keys.items,
        RegistrationKeyKind::Fluid => &keys.fluids,
        RegistrationKeyKind::Effect => &keys.effects,
        RegistrationKeyKind::BlockEntityType => &keys.block_entity_types,
        RegistrationKeyKind::EntityType => &keys.entity_types,
        RegistrationKeyKind::GameEvent => &keys.game_events,
        RegistrationKeyKind::Particle => &keys.particles,
        RegistrationKeyKind::GameRule => &keys.game_rules,
    }
}

/// Whether the lookup epoch is already frozen (including during a later registration attempt).
pub fn parts_installed() -> bool {
    installed().is_some()
}
