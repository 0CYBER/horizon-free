//! Validate the slot-source candidate graph before replacing the loaded snapshot.
use super::{DataStore, Game};
use horizon_client::command::args::Registries;
impl Game<'_> {
    /// The item tags a candidate load would have: the live ones, less those only the loaded
    /// packs define, plus the candidate packs' own. The world tables that publish them are
    /// staged after this check, so a filter may name a tag its own pack adds.
    fn candidate_item_tags(&self, store: &DataStore) -> Vec<String> {
        let pack_tags = |store: &DataStore| -> std::collections::BTreeSet<String> {
            store
                .files
                .keys()
                .filter_map(|key| {
                    let (ns, path) = key.strip_prefix("tags/")?.split_once(':')?;
                    let path = path.strip_prefix("item/")?;
                    Some(horizon_client::command::args::normalize(&format!(
                        "{ns}:{path}"
                    )))
                })
                .collect()
        };
        let base: std::collections::BTreeSet<String> = self
            .res
            .items()
            .item_tags
            .keys()
            .map(|id| horizon_client::command::args::normalize(id))
            .collect();
        let loaded = pack_tags(&self.fns.store);
        let candidate = pack_tags(store);
        let mut tags: std::collections::BTreeSet<String> = self
            .ui
            .registries
            .tags("minecraft:item")
            .iter()
            .filter(|id| base.contains(*id) || !loaded.contains(*id))
            .cloned()
            .collect();
        tags.extend(candidate);
        tags.into_iter().collect()
    }
    /// The registries a candidate load is checked and compiled against: its slot sources and
    /// tags, and its item tags, which the world tables publish only later.
    pub(in crate::game) fn candidate_slot_source_registries(
        &self,
        store: &DataStore,
    ) -> Result<Registries, String> {
        if let Some((id, error)) = store.slot_source_errors.first() {
            return Err(format!("{id}: {error}"));
        }
        let item_tags = self.candidate_item_tags(store);
        let mut registries = self.ui.registries.clone();
        // `Registries::tags` reads an exported registry's entry first (sync_pack_tag_ids).
        if let Some(entry) = registries.by_registry.get_mut("minecraft:item") {
            entry.tags.clone_from(&item_tags);
        }
        registries.item_tags = item_tags;
        let entry = registries
            .by_registry
            .entry("minecraft:slot_source".into())
            .or_default();
        entry.ids = store.slot_sources.keys().cloned().collect();
        entry.tags = store
            .slot_source_tags
            .keys()
            .filter(|id| store.slot_source_tag(id).is_ok())
            .cloned()
            .collect();
        // Ids and tags already walked: shared by every source, so the check stays linear.
        let mut checked = std::collections::BTreeSet::new();
        for (id, value) in &store.slot_sources {
            let (tag, _) = horizon_core::nbt::parse_snbt(&value.to_string())
                .map_err(|error| format!("{id}: {error}"))?;
            // RegistryDataLoader reads a slot_source file with
            // SlotSources.TYPED_CODEC: only an object with a `type` loads, not a
            // holder id or an inline list (which the holder codec would accept).
            if !value.is_object() {
                return Err(format!(
                    "{id}: {}",
                    horizon_client::command::codec::not_map(&tag)
                ));
            }
            horizon_client::command::codec::decode("minecraft:slot_source", &tag, &registries)
                .map_err(|error| format!("{id}: {error}"))?;
            super::super::commands::items::slot_sources::check_references(
                store,
                &serde_json::Value::String(id.clone()),
                &mut std::collections::BTreeSet::new(),
                &mut checked,
            )?;
        }
        Ok(registries)
    }
}
