//! Villager registry indices shared by matching mod loads; no additional packet format.
use horizon_client::mobs::render::VILLAGER_TYPES;
use horizon_client::mobs::{Mob, Species, villager::ResolvedProfessions};
use horizon_protocol::{entity_data::Value, registry::Builtin};
/// VillagerData as the network sends it: the type and the profession by
/// their built-in registry ids, then the level.
pub(super) fn villager_data(m: &mut Mob, data: &[i32], builtin: &Builtin) {
    let &[kind, profession, level] = data else {
        return;
    };
    let index = |registry: &str, id: i32, names: &[&str]| {
        let name = builtin
            .get(registry)?
            .name(id)?
            .strip_prefix("minecraft:")?;
        names.iter().position(|n| *n == name).map(|i| i as u8)
    };
    if let Some(t) = index("minecraft:villager_type", kind, &VILLAGER_TYPES) {
        m.extra.variant = t;
    }
    if let Some(p) = builtin
        .get("minecraft:villager_profession")
        .and_then(|r| r.name(profession))
        .and_then(|name| name.strip_prefix("minecraft:"))
        .and_then(horizon_client::mobs::villager::profession_index)
    {
        m.extra.profession = horizon_client::mobs::villager::ProfessionState::from_index(p);
    }
    m.extra.villager_level = level.max(1);
}

/// Apply the ordinary metadata, then resolve profession slots in the owning load.
pub(super) fn receive(
    m: &mut Mob,
    values: &[(u8, i32, Value)],
    builtin: &Builtin,
    entity: i32,
    table: &ResolvedProfessions,
) -> Option<[f64; 3]> {
    let sound = super::remote_mob_data::apply(m, values, builtin, entity);
    let index = match m.species() {
        Species::Villager => 19,
        Species::ZombieVillager => 20,
        _ => return sound,
    };
    for (field, _, value) in values {
        if *field == index
            && let Value::Ids(data) = value
            && let [_, profession, _] = data.as_slice()
        {
            m.extra.profession = usize::try_from(*profession)
                .ok()
                .and_then(|slot| table.state(slot))
                .unwrap_or_default();
        }
    }
    sound
}

pub(super) fn server_meta(
    m: &Mob,
    hub: &horizon_client::lan::hub::Hub,
    player: Option<i32>,
    time: i64,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
    table: &ResolvedProfessions,
) {
    super::lan::mob_meta(m, hub, player, time, meta, defaults);
    let index = match m.species() {
        Species::Villager => 19,
        Species::ZombieVillager => 20,
        _ => return,
    };
    for (field, _, value) in meta {
        if *field == index
            && let Value::Ids(data) = value
            && data.len() == 3
        {
            // Unknown saved names keep the existing none projection, without changing the save.
            data[1] = table
                .state_index(&m.extra.profession)
                .and_then(|slot| i32::try_from(slot).ok())
                .unwrap_or(0);
        }
    }
}

pub(super) fn unhappy_counter(
    m: &Mob,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    if m.extra.ints[0] > 0 {
        meta.push((
            18,
            horizon_protocol::entity_data::serializer::INT,
            Value::Int(m.extra.ints[0]),
        ));
    }
    defaults.push((
        18,
        horizon_protocol::entity_data::serializer::INT,
        Value::Int(0),
    ));
}

#[cfg(test)]
#[path = "villager_client/tests.rs"]
pub(in crate::game) mod tests;

use std::sync::Arc;
type BuiltinCache = Vec<(std::path::PathBuf, Arc<Builtin>)>;
static BUILTIN: std::sync::Mutex<BuiltinCache> = std::sync::Mutex::new(Vec::new());

/// The prepared resources' `registries.json`, parsed once per root: the
/// file never changes while the game runs, and parsing it took about 3 ms
/// of the frame that joins a server (or opens a single-player world).
/// `None` when it is missing or invalid (not cached: a fixed install can
/// join again).
pub(super) fn builtin_registries(root: &std::path::Path) -> Option<Arc<Builtin>> {
    let mut cache = BUILTIN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, builtin)) = cache.iter().find(|(cached, _)| cached == root) {
        return Some(builtin.clone());
    }
    let builtin = Arc::new(horizon_client::lan::data::with_custom_stats(
        Builtin::find(root).and_then(|p| Builtin::load(&p).ok())?,
    ));
    cache.push((root.to_owned(), builtin.clone()));
    Some(builtin)
}

impl super::Game<'_> {
    pub(super) fn client_professions(&self) -> super::Result<Arc<ResolvedProfessions>> {
        use mod_api::__private::game::RegistrationContextNew;
        let declarations = self.mods.profession_declarations.get_or_init(|| {
            let mut registrar = mod_api::ProfessionDeclarations::default();
            Self::native_register_villager_professions(
                &mod_api::RegistrationContext::new(),
                &mut registrar,
            );
            registrar.into_entries()
        });
        // The client consumes only identity slots. World-dependent POI/trade references
        // are validated by the authoritative server's complete world preparation.
        let table = ResolvedProfessions::prepare(declarations.clone(), |_, _| true)
            .map_err(|errors| format!("client villager professions refused: {errors:?}"))?;
        horizon_client::mobs::render::villager_client::install(
            &table,
            &self
                .res
                .mob_data()
                .villager_hats
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        Ok(Arc::new(table))
    }
}
