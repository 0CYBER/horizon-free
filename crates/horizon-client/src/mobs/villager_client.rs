//! Resource names derived from the identity resolved by the client's profession table.
use super::Cow;
use crate::mobs::villager::ProfessionState;
use crate::mobs::villager::ResolvedProfessions;
use crate::mobs::villager_hats::{Hat, VillagerHats};

/// Resolve the namespace of a profession to the shared atlas's sprite name.
pub(super) fn profession_texture(folder: &str, id: &str) -> Cow<'static, str> {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    if namespace == "minecraft" {
        interned_impl(format_args!("{folder}/profession/{path}"), false)
    } else {
        interned_impl(
            format_args!("{namespace}:{folder}/profession/{path}"),
            false,
        )
    }
}

struct SlotVisual {
    villager: Cow<'static, str>,
    zombie: Cow<'static, str>,
    hat: [Hat; 2],
}

pub struct ProfessionVisuals {
    slots: Vec<SlotVisual>,
    type_hats: [[Hat; super::VILLAGER_TYPES.len()]; 2],
}

impl ProfessionVisuals {
    fn build(table: &ResolvedProfessions, hats: &VillagerHats) -> Self {
        Self {
            slots: table
                .entries()
                .iter()
                .map(|entry| SlotVisual {
                    villager: profession_texture("entity/villager", &entry.id),
                    zombie: profession_texture("entity/zombie_villager", &entry.id),
                    hat: [
                        hats.policy("entity/villager", "profession", &entry.id),
                        hats.policy("entity/zombie_villager", "profession", &entry.id),
                    ],
                })
                .collect(),
            type_hats: ["entity/villager", "entity/zombie_villager"].map(|folder| {
                super::VILLAGER_TYPES.map(|variant| hats.policy(folder, "type", variant))
            }),
        }
    }
}

thread_local! {
    static VISUALS: std::cell::RefCell<ProfessionVisuals> = std::cell::RefCell::new(ProfessionVisuals::build(&ResolvedProfessions::vanilla(), &VillagerHats::default()));
}

pub fn install(table: &ResolvedProfessions, hats: &VillagerHats) {
    VISUALS.with(|cache| *cache.borrow_mut() = ProfessionVisuals::build(table, hats));
}

fn slot(state: &ProfessionState) -> Option<usize> {
    match state {
        ProfessionState::Vanilla(i) => Some(usize::from(*i)),
        ProfessionState::Registered { slot, .. } => Some(usize::from(*slot)),
        _ => None,
    }
}

pub(super) fn texture(state: &ProfessionState, zombie: bool) -> Option<Cow<'static, str>> {
    VISUALS.with(|cache| {
        let cache = cache.borrow();
        let visual = cache.slots.get(slot(state)?)?;
        Some(
            if zombie {
                &visual.zombie
            } else {
                &visual.villager
            }
            .clone(),
        )
    })
}

/// `VillagerProfessionLayer`: whether the type layer draws on the parent model with its hat, or on
/// `noHatModel` because the profession's hat covers the type's.
pub(super) fn type_hat_shown(state: &ProfessionState, zombie: bool, variant: u8) -> bool {
    VISUALS.with(|cache| {
        let cache = cache.borrow();
        let Some(visual) = slot(state).and_then(|slot| cache.slots.get(slot)) else {
            return true;
        };
        let species = usize::from(zombie);
        match visual.hat[species] {
            Hat::None => true,
            Hat::Partial => cache.type_hats[species].get(usize::from(variant)) != Some(&Hat::Full),
            Hat::Full => false,
        }
    })
}

/// A name built from parts (`entity/villager/profession/farmer`), kept for the
/// process so that drawing a mob allocates no names each frame: they come from
/// a few tables (types, professions, levels, materials, variants). Past 4096
/// names on a thread (an unknown item's material from a server) a new one comes
/// back owned.
pub(super) fn interned(args: std::fmt::Arguments) -> Cow<'static, str> {
    interned_impl(args, true)
}

fn interned_impl(args: std::fmt::Arguments, bounded: bool) -> Cow<'static, str> {
    use std::fmt::Write;
    thread_local! {
        static NAMES: std::cell::RefCell<(String, crate::fast_hash::FxHashSet<&'static str>)> =
            std::cell::RefCell::default();
    }
    NAMES.with(|names| {
        let (buffer, set) = &mut *names.borrow_mut();
        buffer.clear();
        let _ = buffer.write_fmt(args);
        if let Some(name) = set.get(buffer.as_str()) {
            return Cow::Borrowed(*name);
        }
        if bounded && set.len() >= 4096 {
            return Cow::Owned(buffer.clone());
        }
        let name: &'static str = Box::leak(buffer.as_str().into());
        set.insert(name);
        Cow::Borrowed(name)
    })
}

#[cfg(test)]
#[path = "villager_client_visual_tests.rs"]
mod tests;
