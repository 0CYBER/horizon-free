//! The load-owned villager profession candidate and its vanilla seed.
use super::NONE;
/// Resolved profession metadata in the owning load.
#[derive(Debug)]
pub struct Profession {
    pub id: String,
    pub trade_set: Option<String>,
    pub render_index: u8,
    pub secondary_blocks: std::collections::HashSet<String>,
    pub requested: Vec<String>,
    pub sound: Option<std::sync::Arc<str>>,
    held_names: std::collections::HashSet<String>,
    acquire_names: std::collections::HashSet<String>,
    trade_keys: [Option<String>; 5],
    /// Complete declarative metadata, including explicit vanilla trade-level keys.
    pub definition: Option<mod_api::ProfessionDefinition>,
    held: Option<horizon_world::poi::PoiType>,
    acquirable: Option<horizon_world::poi::PoiType>,
}

/// One immutable candidate generation, with vanilla entries preceding custom entries.
#[derive(Debug)]
pub struct ResolvedProfessions {
    entries: Vec<Profession>,
    by_name: std::collections::HashMap<String, usize>,
    custom_poi: std::collections::HashMap<String, usize>,
    by_poi: [Option<u8>; 13],
    pub pois: horizon_world::poi::PoiSnapshot,
    pub workstation_pois: Vec<Option<horizon_world::poi::PoiIdentity>>,
}
impl ResolvedProfessions {
    pub fn vanilla() -> Self {
        use horizon_world::poi::PoiType as P;
        let mut entries = [
            ("none", None, None),
            ("armorer", Some("armorer"), Some(P::Armorer)),
            ("butcher", Some("butcher"), Some(P::Butcher)),
            ("cartographer", Some("cartographer"), Some(P::Cartographer)),
            ("cleric", Some("cleric"), Some(P::Cleric)),
            ("farmer", Some("farmer"), Some(P::Farmer)),
            ("fisherman", Some("fisherman"), Some(P::Fisherman)),
            ("fletcher", Some("fletcher"), Some(P::Fletcher)),
            (
                "leatherworker",
                Some("leatherworker"),
                Some(P::Leatherworker),
            ),
            ("librarian", Some("librarian"), Some(P::Librarian)),
            ("mason", Some("mason"), Some(P::Mason)),
            ("nitwit", None, None),
            ("shepherd", Some("shepherd"), Some(P::Shepherd)),
            ("toolsmith", Some("toolsmith"), Some(P::Toolsmith)),
            ("weaponsmith", Some("weaponsmith"), Some(P::Weaponsmith)),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (id, trade_set, poi))| Profession {
            id: id.into(),
            trade_set: trade_set.map(str::to_owned),
            definition: Some(vanilla_definition(id, trade_set, poi)),
            render_index: i as u8,
            secondary_blocks: Default::default(),
            requested: vec![],
            sound: None,
            held_names: Default::default(),
            acquire_names: Default::default(),
            trade_keys: Default::default(),
            held: poi,
            acquirable: poi,
        })
        .collect::<Vec<_>>();
        for p in &mut entries {
            p.compile();
        }
        let mut by_poi = [None; 13];
        for p in &entries {
            if let Some(i) = p.acquirable.and_then(P::job_index) {
                assert!(
                    by_poi[i].replace(p.render_index).is_none(),
                    "duplicate vanilla POI"
                );
            }
        }
        let by_name = entries
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("minecraft:{}", p.id), i))
            .collect();
        Self {
            entries,
            by_poi,
            by_name,
            custom_poi: Default::default(),
            pois: Default::default(),
            workstation_pois: Vec::new(),
        }
    }
    pub fn entries(&self) -> &[Profession] {
        &self.entries
    }
    /// Canonical identities in this world's accepted profession table.
    pub fn registry_ids(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }
    pub fn registry_contains(&self, id: &str) -> bool {
        self.by_name.contains_key(id)
    }
    pub fn get(&self, index: u8) -> &Profession {
        &self.entries[index as usize % 15]
    }
    pub fn find(&self, name: &str) -> Option<u8> {
        self.entries[..15]
            .iter()
            .find(|p| p.id == name)
            .map(|p| p.render_index)
    }
    pub(in crate::mobs) fn by_poi(&self, poi: horizon_world::poi::PoiType) -> Option<u8> {
        self.by_poi[poi.job_index()?]
    }
    pub(in crate::mobs) fn held(&self, index: u8, poi: horizon_world::poi::PoiType) -> bool {
        self.entries
            .get(usize::from(index))
            .is_some_and(|p| p.held == Some(poi))
    }
    pub(in crate::mobs) fn acquirable(&self, index: u8, poi: horizon_world::poi::PoiType) -> bool {
        if index == NONE {
            poi.is_job()
        } else {
            self.entries
                .get(usize::from(index))
                .is_some_and(|p| p.acquirable == Some(poi))
        }
    }
}

impl ResolvedProfessions {
    /// Resolve a complete candidate without touching the active table. The caller installs
    /// this value together with its POI and trade resources only after all preparation succeeds.
    pub fn prepare(
        mut declarations: Vec<(String, mod_api::ProfessionDefinition)>,
        mut exists: impl FnMut(&str, &mod_api::Identifier) -> bool,
    ) -> Result<Self, Vec<mod_api::ProfessionRegistrationError>> {
        use mod_api::ProfessionRegistrationError as E;
        use std::collections::BTreeMap;
        declarations
            .sort_by_key(|(owner, definition)| (definition.key.0.to_string(), owner.clone()));
        let mut errors = Vec::new();
        let seed = Self::vanilla();
        let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut pois: BTreeMap<String, Vec<mod_api::ProfessionKey>> = BTreeMap::new();
        for (owner, definition) in &declarations {
            owners
                .entry(definition.key.0.to_string())
                .or_default()
                .push(owner.clone());
            if definition.key.0.namespace() == "minecraft" {
                errors.push(E::ReservedProfession {
                    key: definition.key.clone(),
                });
            }
            for poi in definition.held_poi.iter().chain(&definition.acquirable_poi) {
                if !exists("poi", &poi.0) {
                    errors.push(E::UnknownPoi { key: poi.clone() });
                }
            }
            for poi in &definition.acquirable_poi {
                let entries = pois.entry(poi.0.to_string()).or_default();
                if !entries.contains(&definition.key) {
                    entries.push(definition.key.clone());
                }
            }
            let mut levels = [false; 5];
            for (level, key) in &definition.trades {
                if !(1..=5).contains(level)
                    || std::mem::replace(
                        &mut levels[usize::from(level.saturating_sub(1)).min(4)],
                        true,
                    )
                {
                    errors.push(E::InvalidTradeLevel {
                        key: definition.key.clone(),
                        level: *level,
                    });
                }
                if !exists("trade_set", &key.0) {
                    errors.push(E::UnknownTradeSet { key: key.clone() });
                }
            }
            for key in &definition.requested_items {
                if !exists("item", key) {
                    errors.push(E::UnknownItem { key: key.clone() });
                }
            }
            for key in &definition.secondary_blocks {
                if !exists("block", key) {
                    errors.push(E::UnknownBlock { key: key.clone() });
                }
            }
            if let Some(key) = &definition.work_sound
                && !exists("sound", key)
            {
                errors.push(E::UnknownSound { key: key.clone() });
            }
        }
        for (_, definition) in &declarations {
            for workstation in &definition.workstations {
                if !pois.contains_key(&workstation.poi.0.to_string()) {
                    errors.push(E::UnacquirablePoi {
                        key: workstation.poi.clone(),
                    });
                }
            }
        }
        for (key, mut owners) in owners {
            if owners.len() > 1 {
                owners.sort();
                errors.push(E::DuplicateProfession {
                    key: mod_api::ProfessionKey(
                        mod_api::Identifier::parse(&key).expect("validated identifier"),
                    ),
                    owners,
                });
            }
        }
        for (poi, mut professions) in pois {
            if let Some(index) =
                horizon_world::poi::PoiType::from_id(&poi).and_then(|poi| seed.by_poi(poi))
            {
                professions.push(mod_api::ProfessionKey(
                    mod_api::Identifier::parse(&format!("minecraft:{}", seed.get(index).id))
                        .expect("vanilla identity"),
                ));
            }
            if professions.len() > 1 {
                professions.sort_by_key(|p| p.0.to_string());
                errors.push(E::OverlappingPoi {
                    poi: mod_api::PoiKey(
                        mod_api::Identifier::parse(&poi).expect("validated identifier"),
                    ),
                    professions,
                });
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut candidate = seed;
        for (_, definition) in declarations {
            let index = candidate.entries.len();
            candidate
                .by_name
                .insert(definition.key.0.to_string(), index);
            for poi in &definition.acquirable_poi {
                candidate.custom_poi.insert(poi.0.to_string(), index);
            }
            candidate.entries.push(Profession {
                id: definition.key.0.to_string(),
                trade_set: None,
                render_index: 0,
                secondary_blocks: Default::default(),
                requested: vec![],
                sound: None,
                held_names: Default::default(),
                acquire_names: Default::default(),
                trade_keys: Default::default(),
                held: None,
                acquirable: None,
                definition: Some(definition),
            });
        }
        for p in &mut candidate.entries {
            p.compile();
        }
        Ok(candidate)
    }

    /// Resolve a qualified saved identity in this load, without numeric truncation.
    pub fn resolve(&self, id: &str) -> Option<usize> {
        self.by_name.get(id).copied()
    }

    /// Metadata by an index scoped to this table.
    pub fn entry(&self, index: usize) -> Option<&Profession> {
        self.entries.get(index)
    }

    /// Resolve a custom workstation's acquisition identity in constant time.
    pub fn acquiring(&self, poi: &str) -> Option<usize> {
        self.custom_poi.get(poi).copied()
    }
}

/// A resource owner's active immutable table. Preparation never mutates this holder;
/// swapping it belongs to the same root commit as the dependent POI and trade tables.
#[derive(Debug)]
pub struct ProfessionRegistry {
    active: std::sync::Arc<ResolvedProfessions>,
}
impl Default for ProfessionRegistry {
    fn default() -> Self {
        Self {
            active: std::sync::Arc::new(ResolvedProfessions::vanilla()),
        }
    }
}
impl ProfessionRegistry {
    /// The complete active table for the owning resource load.
    pub fn active(&self) -> &std::sync::Arc<ResolvedProfessions> {
        &self.active
    }

    /// Publish a successfully prepared candidate, retaining the old table on refusal.
    /// The owning resource transaction calls this only when its dependent tables commit.
    pub fn publish(
        &mut self,
        candidate: Result<ResolvedProfessions, Vec<mod_api::ProfessionRegistrationError>>,
    ) -> Result<(), Vec<mod_api::ProfessionRegistrationError>> {
        self.active = std::sync::Arc::new(candidate?);
        Ok(())
    }
}

#[cfg(test)]
#[path = "villager_professions_tests.rs"]
mod tests;

fn vanilla_definition(
    id: &str,
    trade_set: Option<&str>,
    poi: Option<horizon_world::poi::PoiType>,
) -> mod_api::ProfessionDefinition {
    use mod_api::{Identifier, PoiKey, ProfessionDefinition, ProfessionKey, TradeSetKey};
    let key = |name: &str| Identifier::parse(name).expect("vanilla resource identity");
    let job = poi
        .map(|p| PoiKey(key(p.id())))
        .into_iter()
        .collect::<Vec<_>>();
    ProfessionDefinition {
        workstations: vec![],
        key: ProfessionKey(key(&format!("minecraft:{id}"))),
        held_poi: job.clone(),
        acquirable_poi: job,
        requested_items: if id == "farmer" {
            [
                "minecraft:wheat",
                "minecraft:wheat_seeds",
                "minecraft:beetroot_seeds",
                "minecraft:bone_meal",
            ]
            .into_iter()
            .map(key)
            .collect()
        } else {
            vec![]
        },
        secondary_blocks: if id == "farmer" {
            vec![key("minecraft:farmland")]
        } else {
            vec![]
        },
        work_sound: trade_set.map(|name| key(&format!("minecraft:entity.villager.work_{name}"))),
        trades: trade_set
            .map(|name| {
                (1..=5)
                    .map(|level| {
                        (
                            level,
                            TradeSetKey(key(&format!("minecraft:{name}/level_{level}"))),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

impl ResolvedProfessions {
    /// Resolve stored names only against their owning world's current table.
    pub fn state_index(&self, state: &super::ProfessionState) -> Option<usize> {
        match state {
            super::ProfessionState::None => Some(0),
            super::ProfessionState::Vanilla(i) => (usize::from(*i) < 15).then_some(usize::from(*i)),
            super::ProfessionState::Unresolved(id) => self.resolve(id),
            super::ProfessionState::Registered { id, .. } => self.resolve(id),
        }
    }
    pub fn state(&self, index: usize) -> Option<super::ProfessionState> {
        self.entry(index).map(|p| {
            if index < 15 {
                super::ProfessionState::from_index(index as u8)
            } else {
                super::ProfessionState::Registered {
                    id: p.id.as_str().into(),
                    slot: u16::try_from(index).expect("profession slot"),
                }
            }
        })
    }
    pub fn job(&self, poi: horizon_world::poi::PoiIdentity) -> Option<usize> {
        match poi {
            horizon_world::poi::PoiIdentity::Vanilla(p) => self.by_poi(p).map(usize::from),
            custom => self.acquiring(self.pois.name(custom)?),
        }
    }
    pub fn accepts(&self, index: usize, poi: horizon_world::poi::PoiIdentity, held: bool) -> bool {
        if let horizon_world::poi::PoiIdentity::Vanilla(p) = poi
            && index < 15
        {
            return if held {
                self.held(index as u8, p)
            } else {
                self.acquirable(index as u8, p)
            };
        }
        if index == 0 && !held {
            return self.job(poi).is_some();
        }
        let Some(entry) = self.entry(index) else {
            return false;
        };
        let Some(name) = self.pois.name(poi) else {
            return false;
        };
        if held {
            entry.held_names.contains(name)
        } else {
            entry.acquire_names.contains(name)
        }
    }
    pub fn trade_key(&self, profession: &str, level: i32) -> Option<&str> {
        let index = self
            .resolve(profession)
            .or_else(|| self.find(profession).map(usize::from))?;
        let level = usize::try_from(level).ok()?.checked_sub(1)?;
        self.entry(index)?.trade_keys.get(level)?.as_deref()
    }
}

impl Profession {
    fn compile(&mut self) {
        let Some(d) = &self.definition else {
            return;
        };
        self.held_names = d.held_poi.iter().map(|p| p.0.to_string()).collect();
        self.acquire_names = d.acquirable_poi.iter().map(|p| p.0.to_string()).collect();
        self.secondary_blocks = d.secondary_blocks.iter().map(ToString::to_string).collect();
        self.requested = d.requested_items.iter().map(ToString::to_string).collect();
        self.sound = d.work_sound.as_ref().map(|id| {
            if id.namespace() == "minecraft" {
                id.path().into()
            } else {
                id.to_string().into()
            }
        });
        for (level, key) in &d.trades {
            if (1..=5).contains(level) {
                self.trade_keys[usize::from(*level) - 1] = Some(key.0.to_string());
            }
        }
    }
}

#[path = "villager_profession_save.rs"]
pub mod save;
