//! The map decoration types native mods register (`map_decoration_types`, NeoForge's
//! `Registries.MAP_DECORATION_TYPE`), frozen once at start. A type's id and sprite live for the
//! rest of the process, as the vanilla table's do, so a decoration holds them as `&'static str`.

use mod_api::{Identifier, MapDecorationType};
use std::collections::HashMap;
use std::sync::OnceLock;

/// What the game keeps of a registered type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    id: &'static str,
    sprite: &'static str,
    show_on_item_frame: bool,
}

/// The registered types by id.
#[derive(Debug, Default)]
pub struct MapDecorationTypes {
    by_id: HashMap<&'static str, Entry>,
    by_sprite: HashMap<&'static str, Entry>,
}

static TYPES: OnceLock<MapDecorationTypes> = OnceLock::new();

/// Freezes the types native mods registered, once at start. `vanilla` says whether a sprite is
/// one the game draws a vanilla type with: a mod type may not take it over.
pub fn register(types: Vec<MapDecorationType>, vanilla: impl Fn(&str) -> bool) {
    if TYPES.get().is_some() {
        log::warn!("Map decoration types were registered already; these are left out");
        return;
    }
    // A second thread racing this one finds the table set and drops its own, as before.
    let _ = TYPES.set(MapDecorationTypes::place(types, vanilla));
}

/// The registered types: none until `register` runs.
pub fn registered() -> &'static MapDecorationTypes {
    static NONE: OnceLock<MapDecorationTypes> = OnceLock::new();
    TYPES
        .get()
        .unwrap_or_else(|| NONE.get_or_init(MapDecorationTypes::default))
}

impl MapDecorationTypes {
    /// Registers each type in push order. An id that is not `<namespace>:<path>` in a mod's
    /// namespace, is registered already, or has an empty or already used sprite (a vanilla
    /// type's included, which `vanilla` recognises) is logged and left out.
    pub fn place(registered: Vec<MapDecorationType>, vanilla: impl Fn(&str) -> bool) -> Self {
        let mut types = Self::default();
        for MapDecorationType {
            id,
            sprite,
            show_on_item_frame,
        } in registered
        {
            let own =
                Identifier::parse_namespaced(&id).is_some_and(|id| id.namespace() != "minecraft");
            if !own {
                log::warn!(
                    "Map decoration type {id:?} is not <namespace>:<path> in a mod's namespace"
                );
            } else if sprite.is_empty() {
                log::warn!("Map decoration type {id} has no sprite; it is left out");
            } else if types.by_id.contains_key(id.as_str()) {
                log::warn!("Map decoration type {id} is registered twice; the first is kept");
            } else if vanilla(&sprite) || types.by_sprite.contains_key(sprite.as_str()) {
                log::warn!(
                    "Map decoration type {id} reuses the sprite {sprite} of another; it is left out"
                );
            } else {
                let entry = Entry {
                    id: Box::leak(id.into_boxed_str()),
                    sprite: Box::leak(sprite.into_boxed_str()),
                    show_on_item_frame,
                };
                types.by_id.insert(entry.id, entry);
                types.by_sprite.insert(entry.sprite, entry);
            }
        }
        types
    }

    /// The sprite of the type `id` (`mymod:treasure`).
    pub fn sprite(&self, id: &str) -> Option<&'static str> {
        self.by_id.get(id).map(|e| e.sprite)
    }

    /// The id of the type drawn with `sprite`.
    pub fn id_of(&self, sprite: &str) -> Option<&'static str> {
        self.by_sprite.get(sprite).map(|e| e.id)
    }

    /// `MapDecorationType.showOnItemFrame` of the type drawn with `sprite`.
    pub fn shown_on_frame(&self, sprite: &str) -> Option<bool> {
        self.by_sprite.get(sprite).map(|e| e.show_on_item_frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(id: &str, sprite: &str, frame: bool) -> MapDecorationType {
        MapDecorationType {
            id: id.into(),
            sprite: sprite.into(),
            show_on_item_frame: frame,
        }
    }

    #[test]
    fn a_mod_type_is_found_by_id_and_by_sprite() {
        let types = MapDecorationTypes::place(
            vec![
                declared("mymod:treasure", "mymod:gui/treasure", true),
                declared("mymod:camp", "mymod:gui/camp", false),
            ],
            |_| false,
        );
        assert_eq!(types.sprite("mymod:treasure"), Some("mymod:gui/treasure"));
        assert_eq!(types.id_of("mymod:gui/camp"), Some("mymod:camp"));
        assert_eq!(types.shown_on_frame("mymod:gui/treasure"), Some(true));
        assert_eq!(types.shown_on_frame("mymod:gui/camp"), Some(false));
        assert_eq!(types.sprite("mymod:missing"), None);
        assert_eq!(types.shown_on_frame("map/decorations/player"), None);
    }

    #[test]
    fn bad_and_duplicate_types_are_left_out_and_the_first_wins() {
        let types = MapDecorationTypes::place(
            vec![
                declared("minecraft:red_marker", "mymod:gui/a", true),
                declared("treasure", "mymod:gui/b", true),
                declared("mymod:empty", "", true),
                declared("mymod:one", "mymod:gui/one", true),
                declared("mymod:one", "mymod:gui/two", false),
                declared("mymod:other", "mymod:gui/one", false),
                declared("mymod:thief", "map/decorations/player", true),
            ],
            |sprite| sprite == "map/decorations/player",
        );
        assert_eq!(types.id_of("map/decorations/player"), None);
        assert_eq!(types.by_id.len(), 1);
        assert_eq!(types.sprite("mymod:one"), Some("mymod:gui/one"));
        assert_eq!(types.shown_on_frame("mymod:gui/one"), Some(true));
    }

    #[test]
    fn with_no_mod_types_nothing_is_found() {
        let types = MapDecorationTypes::default();
        assert_eq!(types.sprite("mymod:treasure"), None);
        assert_eq!(types.id_of("mymod:gui/treasure"), None);
    }
}
