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
    /// `instanceof Enemy` for the entities of type `id`: a mob's from its type, true for the
    /// ender dragon, false for every other kind and for an id that names no type.
    pub(in crate::game) fn entity_type_is_enemy(&self, id: &str) -> bool {
        match self.res.mob_types().get(id) {
            Some(kind) => kind.is_enemy(),
            None => id == "minecraft:ender_dragon",
        }
    }
    /// `instanceof Animal` for the entities of type `id`: the mobs whose class extends vanilla's
    /// `Animal`, false for every other kind and for an id that names no type.
    pub(in crate::game) fn entity_type_is_animal(&self, id: &str) -> bool {
        self.res
            .mob_types()
            .get(id)
            .is_some_and(|kind| horizon_client::mobs::drops::is_animal(kind.species))
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

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;

    /// The registry answers by the id as written: a namespaced id is in it, a bare name and an
    /// unknown id are not (`EntityType.byString` finds only registered keys).
    #[test]
    #[ignore = "daily"]
    fn a_type_id_is_looked_up_as_written() {
        with_game(|game| {
            game.spawners.entity_types = Some(["minecraft:zombie".to_owned()].into_iter().collect());
            assert!(game.entity_type_exists("minecraft:zombie"));
            assert!(!game.entity_type_exists("zombie"));
            assert!(!game.entity_type_exists("minecraft:no_such_mob"));
        });
    }

    /// `instanceof Enemy` and `instanceof Animal` follow the classes, not the category: the
    /// hoglin is both, the zombie horse and camel husk are monsters by category but only
    /// `Animal`s, the shulker and the slime are enemies outside `Monster`, the dragon is one
    /// without being a mob here, and the sulfur cube, fish, dolphins, villagers and golems are
    /// neither.
    #[test]
    #[ignore = "daily"]
    fn a_types_enemy_and_animal_follow_its_class() {
        with_game(|game| {
            let classes = |id: &str| {
                (
                    game.entity_type_is_enemy(id),
                    game.entity_type_is_animal(id),
                )
            };
            for id in ["zombie", "creeper", "shulker", "slime", "ghast", "wither"] {
                assert_eq!(classes(&format!("minecraft:{id}")), (true, false), "{id}");
            }
            assert_eq!(classes("minecraft:ender_dragon"), (true, false));
            assert_eq!(classes("minecraft:hoglin"), (true, true));
            for id in [
                "cow",
                "axolotl",
                "zombie_horse",
                "camel_husk",
                "zombie_nautilus",
            ] {
                assert_eq!(classes(&format!("minecraft:{id}")), (false, true), "{id}");
            }
            for id in [
                "sulfur_cube",
                "dolphin",
                "cod",
                "villager",
                "iron_golem",
                "bat",
                "item",
                "player",
            ] {
                assert_eq!(classes(&format!("minecraft:{id}")), (false, false), "{id}");
            }
            assert_eq!(classes("zombie"), (false, false), "an id as written");
        });
    }
}
