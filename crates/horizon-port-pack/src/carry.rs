//! Step 4, carry-over: a jar's `assets/` and `data/` into a folder content
//! mod (API 0.2) for local use. Files are copied, never code; recipes are
//! upgraded from 1.21.1 to 26.3, item models get their `items/` client
//! definitions, and each blockstate gets a vanilla template guessed from its
//! properties until the oracle's facts replace the guess.
use crate::jar::JarInfo;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub copied: usize,
    /// Files outside what a content mod may hold (other namespaces, vanilla
    /// data other than tags), kept in `outside/` for review.
    pub outside: Vec<String>,
    pub recipes_upgraded: usize,
    /// Recipes left as they were: custom types, NeoForge conditions.
    pub recipes_kept: Vec<String>,
    pub item_definitions_added: usize,
    /// Declared blocks and their guessed templates.
    pub blocks: BTreeMap<String, String>,
    /// Blockstates whose properties match no template.
    pub blocks_unknown: Vec<String>,
    pub items: usize,
}

/// Vanilla blocks by their sorted property names.
const TEMPLATES: &[(&str, &str)] = &[
    ("", "minecraft:stone"),
    ("axis", "minecraft:oak_log"),
    ("type,waterlogged", "minecraft:oak_slab"),
    ("facing,half,shape,waterlogged", "minecraft:oak_stairs"),
    ("facing,half,hinge,open,powered", "minecraft:oak_door"),
    (
        "facing,half,open,powered,waterlogged",
        "minecraft:oak_trapdoor",
    ),
    ("east,north,south,waterlogged,west", "minecraft:oak_fence"),
    (
        "east,north,south,up,waterlogged,west",
        "minecraft:cobblestone_wall",
    ),
    ("facing,in_wall,open,powered", "minecraft:oak_fence_gate"),
    ("face,facing,powered", "minecraft:oak_button"),
    ("powered", "minecraft:oak_pressure_plate"),
    ("facing", "minecraft:carved_pumpkin"),
    ("facing,lit", "minecraft:furnace"),
    ("lit", "minecraft:redstone_lamp"),
    ("waterlogged", "minecraft:chain"),
    ("age", "minecraft:wheat"),
    ("distance,persistent,waterlogged", "minecraft:oak_leaves"),
    ("stage", "minecraft:oak_sapling"),
];

pub fn template(properties: &BTreeSet<String>) -> Option<&'static str> {
    let key = properties.iter().cloned().collect::<Vec<_>>().join(",");
    TEMPLATES.iter().find(|(k, _)| *k == key).map(|(_, t)| *t)
}

/// The property names a blockstate file uses.
pub fn properties(blockstate: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |pairs: &str| {
        for pair in pairs.split(',').filter(|p| !p.is_empty()) {
            out.insert(pair.split('=').next().unwrap_or(pair).to_owned());
        }
    };
    for key in blockstate["variants"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
    {
        add(key);
    }
    for part in blockstate["multipart"].as_array().into_iter().flatten() {
        let when = &part["when"];
        let mut conditions: Vec<&Value> = vec![when];
        for or in ["OR", "AND"] {
            conditions.extend(when[or].as_array().into_iter().flatten());
        }
        for c in conditions {
            for (k, _) in c.as_object().into_iter().flatten() {
                if k != "OR" && k != "AND" {
                    out.insert(k.clone());
                }
            }
        }
    }
    out
}

/// A 1.21.1 ingredient (`{"item": id}`, `{"tag": t}` or a list of those) as
/// 26.3's (`"id"`, `"#t"`, `["id", ...]`); `None` for what has no 26.3 form
/// (custom ingredient types, tags inside lists).
pub fn ingredient(v: &Value) -> Option<Value> {
    match v {
        Value::String(_) => Some(v.clone()),
        Value::Object(o) if o.len() == 1 => {
            if let Some(Value::String(id)) = o.get("item") {
                Some(json!(id))
            } else if let Some(Value::String(tag)) = o.get("tag") {
                Some(json!(format!("#{tag}")))
            } else {
                None
            }
        }
        Value::Array(list) => {
            let ids: Option<Vec<Value>> = list
                .iter()
                .map(|x| {
                    x.get("item")
                        .filter(|_| x.as_object().is_some_and(|o| o.len() == 1))
                        .cloned()
                })
                .collect();
            match ids? {
                ids if ids.len() == 1 => ids.into_iter().next(),
                ids => Some(Value::Array(ids)),
            }
        }
        _ => None,
    }
}

/// A vanilla recipe upgraded in place; `false` (unchanged) when it is not
/// a vanilla type or one of its ingredients has no 26.3 form.
pub fn upgrade_recipe(recipe: &mut Value) -> bool {
    let kind = recipe["type"].as_str().unwrap_or("");
    if !kind.starts_with("minecraft:") || recipe.get("neoforge:conditions").is_some() {
        return false;
    }
    let mut out = recipe.clone();
    if let Some(key) = out.get_mut("key").and_then(Value::as_object_mut) {
        for v in key.values_mut() {
            let Some(new) = ingredient(v) else {
                return false;
            };
            *v = new;
        }
    }
    if let Some(list) = out.get_mut("ingredients").and_then(Value::as_array_mut) {
        for v in list.iter_mut() {
            let Some(new) = ingredient(v) else {
                return false;
            };
            *v = new;
        }
    }
    for field in ["ingredient", "base", "addition", "template"] {
        if let Some(v) = out.get_mut(field) {
            let Some(new) = ingredient(v) else {
                return false;
            };
            *v = new;
        }
    }
    *recipe = out;
    true
}

/// Where a jar entry goes: `Some(true)` inside the content mod, `Some(false)`
/// to `outside/`, `None` not carried (code, metadata).
fn place(name: &str, id: &str) -> Option<bool> {
    let mut parts = name.splitn(3, '/');
    let (root, ns, rest) = (parts.next()?, parts.next()?, parts.next()?);
    if !(root == "assets" || root == "data") || rest.is_empty() || name.ends_with('/') {
        return None;
    }
    Some(ns == id || (root == "data" && ns == "minecraft" && rest.starts_with("tags/")))
}

pub fn carry(jar: &Path, info: &JarInfo, out: &Path) -> Result<Report, String> {
    let id = &info.mod_id;
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(format!(
            "mod id {id:?} is not [a-z0-9_]{{1,64}}: rename it before carrying over"
        ));
    }
    let file = fs::File::open(jar).map_err(|e| format!("{}: {e}", jar.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", jar.display()))?;
    let (inside, outside) = (out.join("carry"), out.join("outside"));
    let _ = fs::remove_dir_all(&inside);
    let _ = fs::remove_dir_all(&outside);
    let mut report = Report::default();
    let mut blockstates: BTreeMap<String, Value> = BTreeMap::new();
    let mut item_models = BTreeSet::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        let Some(keep) = place(&name, id) else {
            continue;
        };
        if name.split('/').any(|p| p == ".." || p.is_empty()) {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{name}: {e}"))?;
        let dest = if keep {
            report.copied += 1;
            inside.join(&name)
        } else {
            report.outside.push(name.clone());
            outside.join(&name)
        };
        if keep {
            let rel = name.split_once(&format!("/{id}/")).map_or("", |(_, r)| r);
            if let Some(file) = rel
                .strip_prefix("recipe/")
                .filter(|_| name.starts_with("data/"))
            {
                if let Ok(mut recipe) = serde_json::from_slice::<Value>(&bytes) {
                    if upgrade_recipe(&mut recipe) {
                        report.recipes_upgraded += 1;
                        bytes = serde_json::to_vec_pretty(&recipe).unwrap_or(bytes);
                    } else {
                        report.recipes_kept.push(file.to_owned());
                    }
                }
            } else if let Some(b) = rel
                .strip_prefix("blockstates/")
                .and_then(|b| b.strip_suffix(".json"))
            {
                if let Ok(v) = serde_json::from_slice(&bytes) {
                    blockstates.insert(b.to_owned(), v);
                }
            } else if let Some(m) = rel
                .strip_prefix("models/item/")
                .and_then(|m| m.strip_suffix(".json"))
            {
                item_models.insert(m.to_owned());
            }
        }
        write(&dest, &bytes)?;
    }
    // 1.21.4 moved item models behind `items/<name>.json` definitions.
    for m in &item_models {
        let def = inside.join(format!("assets/{id}/items/{m}.json"));
        if !def.exists() {
            let model =
                json!({"model": {"type": "minecraft:model", "model": format!("{id}:item/{m}")}});
            write(&def, &serde_json::to_vec_pretty(&model).unwrap_or_default())?;
            report.item_definitions_added += 1;
        }
    }
    let decl = inside.join(format!("data/{id}/horizon"));
    for (b, state) in &blockstates {
        match template(&properties(state)) {
            Some(t) => {
                write(
                    &decl.join(format!("block/{b}.json")),
                    json!({"template": t}).to_string().as_bytes(),
                )?;
                report.blocks.insert(b.clone(), t.to_owned());
            }
            None => report.blocks_unknown.push(b.clone()),
        }
    }
    for m in item_models.iter().filter(|m| !blockstates.contains_key(*m)) {
        let item = json!({"template": "minecraft:stick"});
        write(
            &decl.join(format!("item/{m}.json")),
            item.to_string().as_bytes(),
        )?;
        report.items += 1;
    }
    let q = |s: &str| toml::Value::String(s.to_owned()).to_string();
    let name = if info.name.is_empty() { id } else { &info.name };
    let manifest = format!(
        "# Carried over from {jar} by horizon-port-pack, for local use only.\n# Licence: {license}\nid = {}\nname = {}\nversion = {}\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        q(id),
        q(name),
        q(&info.version),
        jar = jar.file_name().unwrap_or_default().to_string_lossy(),
        license = if info.license.is_empty() {
            "not stated"
        } else {
            &info.license
        },
    );
    write(&inside.join("horizon-mod.toml"), manifest.as_bytes())?;
    write(
        &out.join("carry-report.json"),
        &serde_json::to_vec_pretty(&report).unwrap_or_default(),
    )?;
    Ok(report)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn upgrades_ingredients() {
        let mut r = json!({"type": "minecraft:crafting_shaped", "pattern": ["##"],
            "key": {"#": {"item": "gem:gem"}, "s": {"tag": "c:rods/wooden"}},
            "result": {"id": "gem:gem_block", "count": 1}});
        assert!(upgrade_recipe(&mut r));
        assert_eq!(r["key"], json!({"#": "gem:gem", "s": "#c:rods/wooden"}));
        let mut custom = json!({"type": "gem:infuse", "ingredient": {"item": "gem:gem"}});
        assert!(!upgrade_recipe(&mut custom));
        let mut tag_list = json!({"type": "minecraft:smelting",
            "ingredient": [{"tag": "c:ores"}, {"item": "gem:gem"}]});
        assert!(!upgrade_recipe(&mut tag_list));
    }

    #[test]
    fn templates_from_blockstate_properties() {
        let door =
            json!({"variants": {"facing=east,half=lower,hinge=left,open=false,powered=false": {}}});
        assert_eq!(template(&properties(&door)), Some("minecraft:oak_door"));
        let fence = json!({"multipart": [{"apply": {}}, {"when": {"north": "true"}, "apply": {}},
            {"when": {"east": "true"}}, {"when": {"south": "true"}}, {"when": {"west": "true"}},
            {"when": {"OR": [{"waterlogged": "true"}]}}]});
        assert_eq!(template(&properties(&fence)), Some("minecraft:oak_fence"));
        assert_eq!(
            template(&properties(&json!({"variants": {"": {}}}))),
            Some("minecraft:stone")
        );
        let odd = json!({"variants": {"charge=1,mode=fast": {}}});
        assert_eq!(template(&properties(&odd)), None);
    }

    /// A small NeoForge jar carried over is a content mod Horizon loads.
    #[test]
    fn carried_jar_is_a_content_mod() {
        let dir = std::env::temp_dir().join(format!("horizon-port-pack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let jar = dir.join("gem-1.0.jar");
        let mut zip = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
        let files: &[(&str, &str)] = &[
            (
                "META-INF/neoforge.mods.toml",
                "license = \"MIT\"\n[[mods]]\nmodId = \"gem\"\nversion = \"1.0\"\ndisplayName = \"Gem\"\n",
            ),
            ("com/example/gem/Gem.class", "not real bytecode"),
            (
                "assets/gem/blockstates/gem_block.json",
                r#"{"variants": {"": {"model": "gem:block/gem_block"}}}"#,
            ),
            (
                "assets/gem/blockstates/gem_door.json",
                r#"{"variants": {"facing=east,half=lower,hinge=left,open=false,powered=false": {"model": "gem:block/gem_door"}}}"#,
            ),
            (
                "assets/gem/blockstates/gem_core.json",
                r#"{"variants": {"charge=1": {"model": "gem:block/gem_core"}}}"#,
            ),
            (
                "assets/gem/models/item/gem.json",
                r#"{"parent": "minecraft:item/generated", "textures": {"layer0": "gem:item/gem"}}"#,
            ),
            (
                "assets/gem/models/item/gem_block.json",
                r#"{"parent": "gem:block/gem_block"}"#,
            ),
            ("assets/gem/textures/item/gem.png", "png"),
            ("assets/gem/lang/en_us.json", r#"{"item.gem.gem": "Gem"}"#),
            (
                "data/gem/recipe/gem_block.json",
                r####"{"type": "minecraft:crafting_shaped", "pattern": ["###", "###", "###"],
                    "key": {"#": {"item": "gem:gem"}}, "result": {"id": "gem:gem_block", "count": 1}}"####,
            ),
            (
                "data/gem/recipe/infuse.json",
                r#"{"type": "gem:infuse", "ingredient": {"item": "gem:gem"}}"#,
            ),
            (
                "data/gem/loot_table/blocks/gem_block.json",
                r#"{"type": "minecraft:block", "pools": []}"#,
            ),
            (
                "data/minecraft/tags/block/mineable/pickaxe.json",
                r#"{"values": ["gem:gem_block"]}"#,
            ),
            ("data/c/tags/item/gems.json", r#"{"values": ["gem:gem"]}"#),
        ];
        for (name, text) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();

        let info = crate::jar::read(&jar).unwrap();
        assert_eq!(
            (info.mod_id.as_str(), info.license.as_str()),
            ("gem", "MIT")
        );
        assert_eq!(info.census.classes, 1);
        let out = dir.join("work/mods/gem");
        let report = carry(&jar, &info, &out).unwrap();
        assert_eq!(report.copied, 11);
        assert_eq!(report.outside, ["data/c/tags/item/gems.json"]);
        assert_eq!((report.recipes_upgraded, report.recipes_kept.len()), (1, 1));
        assert_eq!(report.item_definitions_added, 2);
        assert_eq!(report.blocks["gem_door"], "minecraft:oak_door");
        assert_eq!(report.blocks["gem_block"], "minecraft:stone");
        assert_eq!(report.blocks_unknown, ["gem_core"]);
        assert_eq!(report.items, 1);
        let carried = out.join("carry");
        assert!(!carried.join("com").exists());
        let recipe: Value = serde_json::from_slice(
            &fs::read(carried.join("data/gem/recipe/gem_block.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(recipe["key"]["#"], "gem:gem");

        // Horizon's own reader accepts it: blocks and items declared, files fenced.
        let package = horizon_mods::Package::read(&carried).unwrap();
        let content = horizon_mods::Content::read(&carried, &package.manifest).unwrap();
        let blocks: Vec<&str> = content.blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(blocks, ["gem:gem_block", "gem:gem_door"]);
        assert_eq!(content.items.len(), 1);
        assert_eq!(content.items[0].id, "gem:gem");
        fs::remove_dir_all(&dir).unwrap();
    }
}
