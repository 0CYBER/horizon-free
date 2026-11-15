//! `RegistryAccess.lookup` and `HolderLookup.listElements` on the current side's actual tables.
use super::Game;
use horizon_client::{command::args::normalized, lan::data::SyncData};
use horizon_core::identifier::Identifier;
use horizon_protocol::registry::{Builtin, Dynamic};
use mod_api::RegistryEntry;
use std::{collections::BTreeMap, sync::Arc};

enum LocalRegistry<'a> {
    Enchantments(&'a horizon_client::enchant::Enchantments),
    Advancements(&'a horizon_client::advancements::Tree),
    Rules(&'a horizon_client::gamerules::RuleRegistry),
    Json(&'a BTreeMap<String, serde_json::Value>),
    HashJson(&'a std::collections::HashMap<String, serde_json::Value>),
    Loot(&'a horizon_client::items::Items),
    Recipes(&'a horizon_client::crafting::Recipes),
    Professions(&'a horizon_client::mobs::villager::ResolvedProfessions),
}
impl LocalRegistry<'_> {
    fn contains(&self, id: &str) -> bool {
        match self {
            Self::Enchantments(table) => table.get(id).is_some(),
            Self::Rules(table) => table.position(id).is_some(),
            Self::Advancements(table) => table.by_id.contains_key(id),
            Self::Json(table) => table.contains_key(id),
            Self::HashJson(table) => table.contains_key(id),
            Self::Loot(table) => table.has_loot_table(id),
            Self::Professions(table) => table.registry_contains(id),
            // RecipeSnapshot is the accepted recipe table, including custom recipe types.
            // Unlike the other owners it has no global name index.
            Self::Recipes(table) => table
                .by_type
                .values()
                .flat_map(|entries| entries.iter())
                .any(|entry| entry.id == id),
        }
    }
    fn ids(&self) -> Vec<&str> {
        let mut ids: Vec<_> = match self {
            Self::Enchantments(table) => table.list.iter().map(|entry| entry.id.as_ref()).collect(),
            Self::Advancements(table) => table.list.iter().map(|entry| entry.id.as_str()).collect(),
            Self::Rules(table) => table
                .available_rules(true)
                .map(|rule| rule.name.as_str())
                .collect(),
            Self::Json(table) => table.keys().map(String::as_str).collect(),
            Self::HashJson(table) => table.keys().map(String::as_str).collect(),
            Self::Loot(table) => table.loot_table_ids().collect(),
            Self::Professions(table) => table.registry_ids().collect(),
            Self::Recipes(table) => table
                .by_type
                .values()
                .flat_map(|entries| entries.iter())
                .map(|entry| entry.id.as_str())
                .collect(),
        };
        // Registry-wide reads promise each identity once, including aliases between recipe types.
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

impl Game<'_> {
    /// The same published provider definitions that `/compute` evaluates.
    pub(in crate::game) fn context_registry(
        &self,
        registry: &str,
    ) -> Option<&std::collections::HashMap<String, serde_json::Value>> {
        self.res.providers().registry(registry)
    }
    fn local_registry(&self, registry: &str) -> Option<LocalRegistry<'_>> {
        Some(match registry {
            "minecraft:enchantment" => {
                LocalRegistry::Enchantments(self.active_enchantment_definitions())
            }
            "minecraft:advancement" => LocalRegistry::Advancements(&self.adv.tree),
            "minecraft:game_rule" => LocalRegistry::Rules(&self.level.rule_registry),
            "minecraft:predicate" => LocalRegistry::Json(&self.fns.store.predicates),
            "minecraft:item_modifier" => LocalRegistry::Json(&self.fns.store.item_modifiers),
            "minecraft:slot_source" => LocalRegistry::Json(&self.fns.store.slot_sources),
            "minecraft:loot_table" => LocalRegistry::Loot(self.items()),
            "minecraft:recipe" => LocalRegistry::Recipes(self.recipes()),
            "minecraft:trade_set" => LocalRegistry::HashJson(&self.stations().trades.sets),
            "minecraft:villager_trade" => LocalRegistry::HashJson(&self.stations().trades.trades),
            "minecraft:villager_profession" => LocalRegistry::Professions(&self.stations().professions),
            _ => LocalRegistry::HashJson(self.context_registry(registry)?),
        })
    }

    fn registry_datapacks(&self, registry: &str) -> Option<Arc<mod_api::DatapackRegistrySet>> {
        if self.is_logical_server() {
            self.mods.datapack_registries.clone()
        } else if self.remote.is_some() {
            // This set has already been filtered by the offer, sync policy and accepted schema.
            horizon_client::datapack_net::received()
                .filter(|set| set.registry_available(registry))
        } else {
            None
        }
    }

    fn registry_sync(&self) -> Option<Arc<SyncData>> {
        self.native_lifecycle.epoch()?;
        if !self.is_logical_server() {
            return None;
        }
        self.lan
            .as_ref()
            .map(|host| host.hub.lan.shared.sync())
            .or_else(|| {
                // The same lazy, frozen/current table the server's next connection receives. Never
                // reconstruct a dynamic registry from the prepared files at a mod call boundary.
                super::lan::sync_data(
                    &self.options.resources,
                    &self.res.core.item_data,
                    &self.pack_files(),
                )
                .ok()
            })
    }

    fn registry_generator_data(&self) -> Option<Arc<horizon_world::worldgen::generator::GeneratorData>> {
        self.end
            .locator()
            .map(|terrain| Arc::clone(terrain.generator_data()))
            .or_else(|| {
                self.res
                    .behaviours()
                    .live_features()
                    .map(|features| Arc::clone(features.data()))
            })
    }

    fn registry_has_entry(&self, registry: &str, id: &str) -> bool {
        if let Some(remote) = &self.remote {
            if registry == "minecraft:game_rule" {
                return self.ui.rules.registry().position(id).is_some();
            }
            return network_id(&remote.builtin, &remote.registries, registry, id).is_some()
                || self
                    .registry_datapacks(registry)
                    .is_some_and(|set| set.contains_entry(registry, id));
        }
        if !self.is_logical_server() || self.native_lifecycle.epoch().is_none() {
            return false;
        }
        if self
            .registry_datapacks(registry)
            .is_some_and(|set| set.contains_entry(registry, id))
        {
            return true;
        }
        if let Some(table) = self.local_registry(registry) {
            return table.contains(id);
        }
        if registry == "minecraft:dimension" {
            return self.level(id).is_some();
        }
        if registry == "minecraft:trial_spawner" {
            return horizon_client::mobs::trial_spawner::registry_contains(id).unwrap_or(false);
        }
        if let Some(found) = self.res.behaviours().live_features()
            .and_then(|features| features.registry_contains(registry, id))
        {
            return found;
        }
        if let Some(found) = self
            .registry_generator_data()
            .and_then(|data| data.registry_contains(registry, id))
        {
            return found;
        }
        self.registry_sync()
            .is_some_and(|sync| network_id(&sync.builtin, &sync.dynamic, registry, id).is_some())
    }

    /// `RegistryAccess.lookup` followed by `HolderGetter.get`: unknown registry/entry is absent.
    pub(super) fn registry_entry(&self, registry: &str, id: &str) -> Option<RegistryEntry> {
        let registry = normalized(registry);
        let id = normalized(id);
        self.registry_has_entry(&registry, &id).then_some(())?;
        Some(RegistryEntry::new(
            Identifier::parse(&registry).ok()?,
            Identifier::parse(&id).ok()?,
        ))
    }

    /// `HolderLookup.listElements`, from the same owners used by `registry_entry`.
    pub(super) fn registry_entries(&self, registry: &str) -> Option<Vec<Identifier>> {
        let registry = normalized(registry);
        if !self.is_logical_server() && self.remote.is_none() {
            return None;
        }
        if self.remote.is_none() {
            self.native_lifecycle.epoch()?;
        }
        if let Some(set) = self.registry_datapacks(&registry)
            && let Ok(entries) = set.entries(&registry)
        {
            return identifiers(entries);
        }
        if let Some(remote) = &self.remote {
            if registry == "minecraft:game_rule" {
                return identifiers(LocalRegistry::Rules(self.ui.rules.registry()).ids());
            }
            return network_entries(&remote.builtin, &remote.registries, &registry);
        }
        if let Some(table) = self.local_registry(&registry) {
            return identifiers(table.ids());
        }
        if registry == "minecraft:dimension" {
            return identifiers(
                std::iter::once(self.dim.id())
                    .chain(self.levels.parked.keys().map(|dimension| dimension.id())),
            );
        }
        if registry == "minecraft:trial_spawner" {
            return identifiers(horizon_client::mobs::trial_spawner::registry_ids()?);
        }
        if let Some(entries) = self.res.behaviours().live_features().and_then(|features| {
            features.registry_ids(&registry)
                .map(|entries| identifiers(entries.iter().map(String::as_str)))
        }) {
            return entries;
        }
        if let Some(entries) = self.registry_generator_data().and_then(|data| {
            data.registry_ids(&registry).map(identifiers)
        }) {
            return entries;
        }
        let sync = self.registry_sync()?;
        network_entries(&sync.builtin, &sync.dynamic, &registry)
    }

    /// `Holder.is(TagKey)`: plain values are resolved again against the current table.
    /// An absent tag is false; an absent registry or entry is None.
    pub(super) fn registry_entry_in_tag(&self, entry: &RegistryEntry, tag: &str) -> Option<bool> {
        let registry = entry.registry.to_string();
        let id = entry.id.to_string();
        if !self.registry_has_entry(&registry, &id) {
            return None;
        }
        let tag = normalized(tag);
        if let Some(set) = self.registry_datapacks(&registry)
            && let Some(found) = set.tag_contains(&registry, &id, &tag)
        {
            return Some(found);
        }
        if let Some(remote) = &self.remote {
            let Some(index) = network_id(&remote.builtin, &remote.registries, &registry, &id)
            else {
                return Some(false);
            };
            return Some(
                remote
                    .registries
                    .tag(&registry, &tag)
                    .is_some_and(|members| members.contains(&index)),
            );
        }
        if registry == "minecraft:enchantment" {
            return Some(
                self.active_enchantment_definitions()
                    .tag(&tag)
                    .iter()
                    .any(|member| member.as_ref() == id),
            );
        }
        if registry == "minecraft:slot_source" {
            return Some(
                self.fns
                    .store
                    .slot_source_tag(&tag)
                    .is_ok_and(|members| members.iter().any(|member| member == &id)),
            );
        }
        if registry == "minecraft:villager_trade" {
            return Some(
                self.stations()
                    .trades
                    .tags
                    .get(tag.as_ref())
                    .is_some_and(|members| members.iter().any(|member| member == &id)),
            );
        }
        let item_tags = match registry.as_str() {
            "minecraft:block" => Some(horizon_client::items::TagRegistry::Block),
            "minecraft:item" => Some(horizon_client::items::TagRegistry::Item),
            "minecraft:fluid" => Some(horizon_client::items::TagRegistry::Fluid),
            _ => None,
        };
        if let Some(kind) = item_tags {
            return Some(
                self.items()
                    .tag_members(kind, &tag)
                    .is_some_and(|members| members.iter().any(|member| member == &id)),
            );
        }
        if let Some(found) = self.res.behaviours().live_features()
            .and_then(|features| features.registry_tag_contains(&registry, &id, &tag))
        {
            return Some(found);
        }
        if let Some(found) = self.registry_generator_data().and_then(|data| {
            data.registry_tag_contains(&registry, &id, &tag)
        }) {
            return Some(found);
        }
        if let Some(tables) = &self.world_tables
            && let Some(kind) = registry.strip_prefix("minecraft:")
            && let Some(tags) = tables.pack_tags.get(kind)
        {
            return Some(
                tags.get(tag.as_ref())
                    .is_some_and(|members| members.contains(&id)),
            );
        }
        let Some(sync) = self.registry_sync() else {
            return Some(false);
        };
        let Some(index) = network_id(&sync.builtin, &sync.dynamic, &registry, &id) else {
            return Some(false);
        };
        Some(
            sync.tags
                .registries
                .iter()
                .find(|tags| tags.registry == registry)
                .and_then(|tags| tags.tags.iter().find(|candidate| candidate.name == tag))
                .is_some_and(|tag| tag.ids.contains(&index)),
        )
    }

    pub(super) fn registry_entry_key_slot(&self, entry: &RegistryEntry) -> Option<u32> {
        if !self.registry_has_entry(&entry.registry.to_string(), &entry.id.to_string()) {
            return None;
        }
        mod_api::__private::registry_entry_key_slot(entry)
    }
}

fn identifiers<'a>(entries: impl IntoIterator<Item = &'a str>) -> Option<Vec<Identifier>> {
    Some(
        entries
            .into_iter()
            .filter_map(|name| Identifier::parse(name).ok())
            .collect(),
    )
}
fn network_id(builtin: &Builtin, dynamic: &Dynamic, registry: &str, id: &str) -> Option<i32> {
    builtin
        .get(registry)
        .and_then(|entries| entries.id(id))
        .or_else(|| dynamic.id(registry, id))
        .or_else(|| {
            (dynamic.custom_components && registry == "minecraft:data_component_type")
                .then(|| horizon_protocol::item::custom::id(id))
                .flatten()
        })
}
fn network_entries(
    builtin: &Builtin,
    dynamic: &Dynamic,
    registry: &str,
) -> Option<Vec<Identifier>> {
    if let Some(ids) = builtin.get(registry) {
        let mut entries = identifiers(
            ids.names
                .iter()
                .filter(|name| !name.is_empty())
                .map(String::as_str),
        )?;
        if dynamic.custom_components && registry == "minecraft:data_component_type" {
            entries.extend(
                horizon_protocol::item::custom::entries()
                    .filter(|(_, name)| ids.id(name).is_none())
                    .filter_map(|(_, name)| Identifier::parse(name).ok()),
            );
        }
        return Some(entries);
    }
    identifiers(
        dynamic
            .registries
            .get(registry)?
            .iter()
            .map(|(name, _)| name.as_str()),
    )
}

/// TagLoader binds mod-registry tags against the already decoded candidate, before publication.
/// No file's name can create an entry: required missing members invalidate that tag only.
pub(super) fn bind_datapack_tags(
    root: &std::path::Path,
    overlay: &horizon_world::loot::PackOverlay<'_>,
    candidate: &mut mod_api::DatapackCandidate,
) -> Result<(), String> {
    for declaration in mod_api::__private::datapack_registry_declarations() {
        let (namespace, path) = declaration
            .key
            .split_once(':')
            .ok_or("invalid mod registry key")?;
        let folder = format!("{namespace}/{path}");
        let known: std::collections::BTreeSet<_> = candidate
            .entries(&declaration.key)
            .map_err(|error| format!("mod registry tags: {error:?}"))?
            .into_iter()
            .collect();
        let tags = load_registry_tags(root, overlay, &folder, |id| known.contains(id))?;
        candidate
            .bind_tags(&declaration.key, tags)
            .map_err(|error| format!("mod registry tags: {error:?}"))?;
    }
    Ok(())
}

fn load_registry_tags(
    root: &std::path::Path,
    overlay: &horizon_world::loot::PackOverlay<'_>,
    folder: &str,
    known: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    use horizon_world::{damage_source::resource_files, worldgen::tags::TagDefinitions};
    let tag_folder = format!("tags/{folder}");
    let mut files: BTreeMap<String, Vec<Vec<u8>>> = resource_files(root, &tag_folder)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|(id, bytes)| (id, vec![bytes]))
        .collect();
    for (id, versions) in overlay.tag_kind(folder) {
        files
            .entry(id)
            .or_default()
            .extend(versions.iter().cloned());
    }
    let mut files: Vec<_> = files.into_iter().collect();
    files.sort_by_cached_key(|(id, _)| {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        (format!("{path}.json"), namespace.to_owned())
    });
    let mut definitions = TagDefinitions::default();
    for (id, versions) in files {
        for bytes in versions {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if !overlay.conditions.allows_id(&value, &tag_folder, &id) {
                continue;
            }
            if let Ok(json) = std::str::from_utf8(&bytes) {
                // TagLoader retains lower-priority members when a tag codec fails.
                let _ = definitions.apply_json(&id, "registry tags", json);
            }
        }
    }
    definitions
        .build(|id, _| known(id).then(|| id.to_owned()))
        .map(|built| built.tags)
        .map_err(|error| error.to_string())
}
