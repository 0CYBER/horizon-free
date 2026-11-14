//! Content sound slots retain dynamic vanilla selection when inherited.
use horizon_mod_package::content::entity_sounds::{EntitySounds, SoundOverride};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, Default)]
pub enum Selection {
    #[default]
    Inherit,
    Id(&'static str),
    Mute,
}
impl Selection {
    /// No lookup or allocation for inherited or muted slots.
    pub fn select(self, inherited: &'static str) -> &'static str {
        match self {
            Self::Inherit => inherited,
            Self::Id(id) => id,
            Self::Mute => "",
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Sounds {
    pub ambient: Selection,
    pub hurt: Selection,
    pub death: Selection,
    pub step: Selection,
}
impl Sounds {
    pub fn load(entity: &str, row: &serde_json::Value) -> crate::Result<Self> {
        let sounds: EntitySounds = match row.get("sounds") {
            None => EntitySounds::default(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|error| format!("{entity}: sounds: {error}"))?,
        };
        sounds.validate(entity)?;
        let resolve = |slot| match slot {
            None => Selection::Inherit,
            Some(SoundOverride::Mute(_)) => Selection::Mute,
            Some(SoundOverride::Id(id)) => Selection::Id(intern_full_id(&id)),
        };
        Ok(Self {
            ambient: resolve(sounds.ambient),
            hurt: resolve(sounds.hurt),
            death: resolve(sounds.death),
            step: resolve(sounds.step),
        })
    }
}
/// Namespace normalization at audio/network boundaries. Full ids are borrowed unchanged.
pub fn full_id(id: &str) -> Cow<'_, str> {
    if id.contains(':') {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(format!("minecraft:{id}"))
    }
}

pub(crate) fn intern_full_id(id: &str) -> &'static str {
    use std::{collections::HashSet, sync::Mutex};
    static IDS: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut ids = IDS.lock().unwrap_or_else(|e| e.into_inner());
    let ids = ids.get_or_insert_with(HashSet::new);
    if let Some(id) = ids.get(id) {
        return id;
    }
    let id = Box::leak(id.to_owned().into_boxed_str());
    ids.insert(id);
    id
}

pub fn ambient(m: &mut super::Mob, w: &super::MobWorld, events: &mut Vec<super::MobEvent>) {
    match m.kind.sound_overrides.ambient {
        Selection::Inherit => super::ai::play_ambient(m, w, events),
        Selection::Id(id) => m.voice(id, events),
        Selection::Mute => {}
    }
}
pub fn voices(m: &super::Mob) -> [&'static str; 3] {
    let inherited = super::ai::inherited_voice_sounds(m);
    let slots = &m.kind.sound_overrides;
    [
        slots.ambient.select(inherited[0]),
        slots.hurt.select(inherited[1]),
        slots.death.select(inherited[2]),
    ]
}
pub fn step(
    m: &super::Mob,
    w: &super::MobWorld,
    pos: [i32; 3],
    state: horizon_world::registry::BlockStateId,
) -> Option<(Cow<'static, str>, f32, f32)> {
    match m.kind.sound_overrides.step {
        Selection::Inherit => super::ai::inherited_step_sound(m, w, pos, state),
        Selection::Id(id) => Some((Cow::Borrowed(id), 0.15, 1.0)),
        Selection::Mute => None,
    }
}

#[cfg(test)]
#[path = "entity_sounds_tests.rs"]
mod tests;
