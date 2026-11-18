//! Shared recipe resource merge, conditions and filters.
use crate::Result;
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::{
    cell::OnceCell,
    collections::{HashMap, HashSet},
    path::Path,
};

/// The items and item tags of the root a recipe load reads, for the
/// `neoforge:item_exists` and `neoforge:tag_empty` conditions. Read on the
/// first condition that asks, so a load without them never reads them.
struct RootItems<'a> {
    root: &'a Path,
    overlay: PackOverlay<'a>,
    facts: OnceCell<Option<(HashSet<String>, HashMap<String, HashSet<String>>)>>,
}

impl RootItems<'_> {
    fn facts(&self) -> Option<&(HashSet<String>, HashMap<String, HashSet<String>>)> {
        self.facts
            .get_or_init(|| {
                let bytes = std::fs::read(self.root.join("items.json")).ok()?;
                let items: HashMap<String, serde::de::IgnoredAny> =
                    serde_json::from_slice(&bytes).ok()?;
                let items: HashSet<String> = items.into_keys().collect();
                // A tag lists entries for items that may not be there (optional ones);
                // vanilla's tag holds only the items that are, so `tag_empty` counts those.
                let mut tags = crate::items::load_tags(self.root, "item", &self.overlay).ok()?;
                for members in tags.values_mut() {
                    members.retain(|id| items.contains(id));
                }
                Some((items, tags))
            })
            .as_ref()
    }
}

impl horizon_world::conditions::ItemFacts for RootItems<'_> {
    fn item_exists(&self, id: &str) -> Option<bool> {
        Some(self.facts()?.0.contains(id))
    }

    fn item_tag_empty(&self, tag: &str) -> Option<bool> {
        Some(self.facts()?.1.get(tag).is_none_or(HashSet::is_empty))
    }
}

/// A recipe's whole id from its name in the tables: vanilla's are kept bare
/// (`oak_planks`), everyone else's whole (`mymod:crusher/iron`). The book,
/// the network copy, the advancements' JSON and brewing know a recipe by
/// this id.
pub fn qualified(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// The recipe files the tables are built from, each id once: vanilla's by
/// bare name, then the content mods' as `<ns>:<name>`, the world's packs'
/// over them (`overlay`), parsed, and only those whose load conditions
/// hold. A server sends some of them to its clients (`recipe_content`).
pub fn sources(root: &Path, overlay: &PackOverlay) -> Result<Vec<(String, Value)>> {
    // Vanilla's recipes by bare name, then content mods' as `<ns>:<name>`.
    let mut sources: Vec<(String, Vec<u8>)> = Vec::new();
    // Load conditions: `mod_loaded` asks which mods' data is
    // in this root (`minecraft`, always, and the content mods').
    // `mod_loaded` asks which mods are loaded, not which namespaces
    // they own: a mod that also owns a namespace doesn't make that
    // namespace another mod.
    let inventory = horizon_mod_package::enabled::EnabledMods::read(root)?;
    let conditions = overlay
        .conditions
        .with_mod_inventory(inventory.server_ids())
        .with_diagnostics();
    for file in &horizon_world::json_folder::read(&root.join("data/minecraft/recipe"))?.files {
        sources.push((file.name.clone(), file.bytes()?.to_vec()));
    }
    for namespace in crate::content_namespaces::namespaces(root) {
        let dir = root.join(format!("data/{namespace}/recipe"));
        if dir.is_dir() {
            for file in &horizon_world::json_folder::read(&dir)?.files {
                sources.push((format!("{namespace}:{}", file.name), file.bytes()?.to_vec()));
            }
        }
    }
    // The world's packs' recipes over the game's own: the top pack's
    // file for each id replaces it (`SimpleJsonResourceReloadListener`
    // sees one resource per id, the resource stack's top).
    for (id, files) in overlay.kind("recipe") {
        let Some((ns, path)) = id.split_once(':') else {
            continue;
        };
        let name = if ns == "minecraft" {
            path.to_owned()
        } else {
            id.to_owned()
        };
        let bytes = PackOverlay::top(files).to_vec();
        match sources.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = bytes,
            None => sources.push((name, bytes)),
        }
    }
    let filters = crate::recipe_filters::RecipeFilters::load(root, overlay);
    let items = RootItems {
        root,
        overlay: *overlay,
        facts: OnceCell::new(),
    };
    let mut parsed = Vec::with_capacity(sources.len());
    for (name, bytes) in sources {
        // Vanilla's RecipeManager logs a file that does not parse and
        // goes on ("Couldn't parse data file").
        let mut v: Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Horizon: recipe {name} not loaded: {e}");
                continue;
            }
        };
        // A type without a namespace, or with an empty one, is vanilla's:
        // `Identifier.parse` reads `crafting_shaped` and `:crafting_shaped` as
        // `minecraft:crafting_shaped`.
        if let Some(kind) = v["type"].as_str() {
            let (namespace, path) = kind.split_once(':').unwrap_or(("", kind));
            if namespace.is_empty() {
                v["type"] = Value::String(format!("minecraft:{path}"));
            }
        }
        // A file whose load conditions don't hold is silently absent.
        if !conditions.allows_id_with_items(&v, "recipe", &qualified(&name), Some(&items)) {
            continue;
        }
        // The data's recipe filters, over the final set.
        if !filters.apply(&qualified(&name), &mut v) {
            continue;
        }
        parsed.push((name, v));
    }
    conditions.check_resource_errors("recipe")?;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    /// A content mod's recipes guarded by NeoForge's item conditions are
    /// answered from the root's items and item tags, and the load goes on.
    #[test]
    fn recipes_with_item_conditions_load() {
        let root =
            std::env::temp_dir().join(format!("horizon-recipe-conditions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let write = |path: &str, v: &Value| {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, serde_json::to_vec(v).unwrap()).unwrap();
        };
        let enabled = horizon_mod_package::enabled::EnabledMods::new([(
            "gem".to_owned(),
            horizon_mod_package::manifest::Side::Both,
        )]);
        write(
            horizon_mod_package::enabled::MARKER,
            &serde_json::to_value(&enabled).unwrap(),
        );
        write(crate::content_namespaces::MARKER, &json!(["gem"]));
        write(
            "items.json",
            &json!({"minecraft:stick": {}, "gem:ruby": {}}),
        );
        write(
            "data/c/tags/item/ingots/copper.json",
            &json!({"values": ["minecraft:stick"]}),
        );
        write(
            "data/c/tags/item/ingots/silver.json",
            &json!({"values": [{"id": "gem:silver_ingot", "required": false}]}),
        );
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        for (name, condition) in [
            (
                "ruby",
                json!({"type": "neoforge:item_exists", "item": "gem:ruby"}),
            ),
            (
                "topaz",
                json!({"type": "neoforge:item_exists", "item": "gem:topaz"}),
            ),
            (
                "from_copper",
                json!({"type": "neoforge:not",
                "value": {"type": "neoforge:tag_empty", "tag": "c:ingots/copper"}}),
            ),
            (
                "copper_fallback",
                json!({"type": "neoforge:tag_empty", "tag": "c:ingots/copper"}),
            ),
            (
                "tin_fallback",
                json!({"type": "neoforge:tag_empty", "tag": "c:ingots/tin"}),
            ),
            (
                "silver_fallback",
                json!({"type": "neoforge:tag_empty", "tag": "c:ingots/silver"}),
            ),
            ("never", json!({"type": "neoforge:false"})),
        ] {
            let recipe = json!({"type": "minecraft:crafting_shapeless",
                "ingredients": ["minecraft:stick"], "result": {"id": "gem:ruby"},
                "neoforge:conditions": [condition]});
            write(&format!("data/gem/recipe/{name}.json"), &recipe);
        }
        let mut loaded: Vec<String> = sources(&root, &PackOverlay::NONE)
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        loaded.sort();
        assert_eq!(
            loaded,
            [
                "gem:from_copper",
                "gem:ruby",
                "gem:silver_fallback",
                "gem:tin_fallback"
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }
}
