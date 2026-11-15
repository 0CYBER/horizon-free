//! API-owned live registration requests, observed only during authoring.
use super::*;
use std::sync::atomic::AtomicUsize;

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct ActiveRegistration;
impl ActiveRegistration {
    pub(super) fn enter() -> Self {
        ACTIVE.fetch_add(1, Ordering::Relaxed);
        Self
    }
}
impl Drop for ActiveRegistration {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
    }
}

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

pub(crate) const REGISTRATION_ACCESS: RegistrationAccess = RegistrationAccess {
    active: &ACTIVE,
    contains: |kind, slot| {
        with_registering_keys(|keys| keys.map(|keys| names(keys, kind).id(slot).is_some()))
    },
    name: |kind, slot| {
        with_registering_keys(|keys| {
            keys.and_then(|keys| names(keys, kind).id(slot).map(str::to_owned))
        })
    },
    in_scope: || REGISTERING_KEYS.with_borrow(Option::is_some),
    install: install_parts,
};
