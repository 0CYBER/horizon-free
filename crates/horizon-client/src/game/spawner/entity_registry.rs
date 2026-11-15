//! Shared type existence and dimensions from the existing prepared registry.
use super::Game;
use std::collections::HashSet;

impl Game<'_> {
    /// `EntityType.byString`: the type is in the registry.
    pub(in crate::game) fn entity_type_exists(&self, id: &str) -> bool {
        self.spawners
            .entity_types
            .as_ref()
            .is_some_and(|t| t.contains(id))
    }
    /// Whether `entity_type_exists` has a registry to answer from.
    pub(in crate::game) fn entity_types_loaded(&self) -> bool {
        self.spawners
            .entity_types
            .as_ref()
            .is_some_and(|t| !t.is_empty())
    }
    pub(in crate::game) fn load_entity_types(&mut self) {
        if self.spawners.entity_types.is_some() {
            return;
        }
        let path = self.options.resources.join("entity-types.json");
        let registry = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        let mut types = HashSet::new();
        if let Some(entries) = registry.as_ref().and_then(|v| v.as_object()) {
            for (kind, data) in entries {
                let kind = horizon_client::command::args::normalize(kind);
                types.insert(kind.clone());
                if let (Some(width), Some(height)) =
                    (data["width"].as_f64(), data["height"].as_f64())
                {
                    self.spawners
                        .entity_dimensions
                        .insert(kind, [width as f32, height as f32]);
                }
            }
        }
        self.spawners.entity_types = Some(types);
    }
    pub(in crate::game) fn registered_entity_dimensions(&self, kind: &str) -> Option<[f32; 2]> {
        self.spawners.entity_dimensions.get(kind).copied()
    }
}

/// Pinned EntityTypes Builder.noSave registrations. Player additionally has a
/// null factory; the other three are live entities with no saved type id.
pub(in crate::game) fn entity_type_saves(kind: &str) -> bool {
    !matches!(
        kind,
        "minecraft:player"
            | "minecraft:fishing_bobber"
            | "minecraft:leash_knot"
            | "minecraft:lightning_bolt"
    )
}
