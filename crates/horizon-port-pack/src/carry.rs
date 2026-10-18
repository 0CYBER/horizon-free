//! Step 4, carry-over: a jar's `assets/` and `data/` into a folder content
//! mod (API 0.2) for local use. Files are copied, never code; recipes are
//! upgraded from 1.21.1 to 26.3, item models get their `items/` client
//! definitions, and each blockstate gets a vanilla template guessed from its
//! properties until the oracle's facts replace the guess.
use crate::{jar::JarInfo, oracle::Facts};
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
    /// Files of the mod's own content the game would refuse, kept in
    /// `outside/` too, with why: a path vanilla does not read (upper case),
    /// JSON that does not parse, a vanilla tag the mod replaces.
    pub unreadable: BTreeMap<String, String>,
    pub recipes_upgraded: usize,
    pub loot_tables_upgraded: usize,
    /// Blocks whose jar has no loot table but drop themselves in the oracle:
    /// the mod registers that table in code, and carry-over writes it.
    pub loot_tables_added: usize,
    /// Worldgen and structure files (`data/<id>/worldgen/`, `structure/`).
    pub worldgen: usize,
    /// Recipes left as they were: custom types, NeoForge conditions.
    pub recipes_kept: Vec<String>,
    pub item_definitions_added: usize,
    /// Templates and properties came from the NeoForge oracle's facts, not
    /// from guesses.
    pub oracle: bool,
    /// Declared blocks and their templates.
    pub blocks: BTreeMap<String, String>,
    /// What a declared block's template does not reproduce (oracle only).
    pub differences: BTreeMap<String, Vec<String>>,
    /// Blocks left out, and why.
    pub blocks_unknown: BTreeMap<String, String>,
    pub items: usize,
    /// Items left out, and why (oracle only).
    pub items_unknown: BTreeMap<String, String>,
    /// Convention (`c:`) tags the recipes use, filled in from the oracle.
    pub convention_tags: usize,
    /// Structure, placement and pool element types rewritten as vanilla's,
    /// and those left as they are.
    pub worldgen_types: crate::worldgen::Upgrade,
    /// The loader's convention biome tags the structures name, carried.
    pub convention_biome_tags: usize,
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
    ("east,north,south,waterlogged,west", "minecraft:glass_pane"),
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

/// A guess without the oracle: the template whose properties include the
/// blockstate's with the fewest left over (a blockstate may leave out what
/// its models ignore, such as `powered` or `waterlogged`), the block's name
/// breaking ties (`_pane`, `_gate`, `_trapdoor`).
pub fn template(name: &str, properties: &BTreeSet<String>) -> Option<&'static str> {
    let fits = TEMPLATES.iter().filter_map(|(k, t)| {
        let have: BTreeSet<&str> = k.split(',').filter(|p| !p.is_empty()).collect();
        properties
            .iter()
            .all(|p| have.contains(p.as_str()))
            .then(|| (have.len() - properties.len(), *t))
    });
    let hint = |t: &str| {
        let kind = t.rsplit('_').next().unwrap_or(t);
        !name.ends_with(&format!("_{kind}"))
    };
    fits.min_by_key(|&(extra, t)| (extra, hint(t)))
        .map(|(_, t)| t)
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

/// A 1.21.1 loot table in 26.3's shape, in place: `conditions` lists become
/// one `condition` (`all_of` for several), `functions` become `modifier`,
/// and conditions and functions name their kind in `type`. A tool match on
/// silk touch alone, or on shears alone, becomes 26.3's
/// `minecraft:tool/can_silk_touch` or `minecraft:tool/can_shear`.
pub fn upgrade_loot(v: &mut Value) {
    match v {
        Value::Array(a) => a.iter_mut().for_each(upgrade_loot),
        Value::Object(o) => {
            if let Some(Value::Array(mut list)) = o.remove("conditions") {
                list.iter_mut().for_each(upgrade_condition);
                let one = match <[Value; 1]>::try_from(list) {
                    Ok([c]) => c,
                    Err(list) => json!({"type": "minecraft:all_of", "terms": list}),
                };
                o.insert("condition".to_owned(), one);
            }
            if let Some(Value::Array(mut list)) = o.remove("functions") {
                for f in &mut list {
                    rename_kind(f, "function");
                    upgrade_loot(f);
                }
                o.insert("modifier".to_owned(), Value::Array(list));
            }
            for (k, x) in o.iter_mut() {
                if k != "condition" && k != "modifier" {
                    upgrade_loot(x);
                }
            }
        }
        _ => {}
    }
}

/// `{"<key>": kind, ...}` as `{"type": kind, ...}`.
fn rename_kind(v: &mut Value, key: &str) {
    if let Some(o) = v.as_object_mut()
        && !o.contains_key("type")
        && let Some(kind) = o.remove(key)
    {
        o.insert("type".to_owned(), kind);
    }
}

/// One 1.21.1 loot condition in 26.3's shape.
fn upgrade_condition(c: &mut Value) {
    rename_kind(c, "condition");
    if let Some(term) = c.get_mut("term") {
        upgrade_condition(term);
    }
    if let Some(Value::Array(terms)) = c.get_mut("terms") {
        terms.iter_mut().for_each(upgrade_condition);
    }
    if c["type"] == "minecraft:match_tool"
        && let Some(named) = named_tool_condition(&c["predicate"])
    {
        *c = json!(named);
    }
}

/// 26.3's named tool conditions for the 1.21.1 predicates they replace.
fn named_tool_condition(predicate: &Value) -> Option<&'static str> {
    let keys: Vec<&str> = predicate.as_object()?.keys().map(String::as_str).collect();
    match keys.as_slice() {
        ["predicates"] => {
            let list = predicate["predicates"]["minecraft:enchantments"].as_array()?;
            let [e] = list.as_slice() else {
                return None;
            };
            let silk = e["enchantments"] == "minecraft:silk_touch"
                && e["levels"]["min"].as_u64().unwrap_or(1) <= 1
                && e["levels"].get("max").is_none()
                && predicate["predicates"].as_object()?.len() == 1;
            silk.then_some("minecraft:tool/can_silk_touch")
        }
        ["items"] => {
            let items = &predicate["items"];
            let shears = items == "minecraft:shears" || *items == json!(["minecraft:shears"]);
            shears.then_some("minecraft:tool/can_shear")
        }
        _ => None,
    }
}

/// Where a jar entry goes: `Some(true)` inside the content mod (its own
/// namespace, additions to vanilla and convention tags), `Some(false)`
/// to `outside/`, `None` not carried (code, metadata).
fn place(name: &str, id: &str) -> Option<bool> {
    let mut parts = name.splitn(3, '/');
    let (root, ns, rest) = (parts.next()?, parts.next()?, parts.next()?);
    if !(root == "assets" || root == "data") || rest.is_empty() || name.ends_with('/') {
        return None;
    }
    let shared_tags =
        root == "data" && (ns == "minecraft" || ns == "c") && rest.starts_with("tags/");
    Some(ns == id || shared_tags)
}

/// Why the game would refuse a file of the mod's own content, if it would.
/// Vanilla skips such files one by one (an invalid resource path, a data
/// file that does not parse); Horizon's content loader refuses the whole
/// set, so they stay out of the carry.
fn unreadable(name: &str, bytes: &mut Vec<u8>) -> Option<String> {
    if let Err(e) = horizon_mods::content::check_path(name) {
        return Some(e);
    }
    if !(name.ends_with(".json") || name.ends_with(".mcmeta")) {
        return None;
    }
    // A byte-order mark: Horizon's JSON readers take the file without it.
    if bytes.starts_with(b"\xEF\xBB\xBF") {
        bytes.drain(..3);
    }
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return Some(format!("does not parse as JSON: {e}")),
    };
    let shared_tag = name.starts_with("data/minecraft/tags/") || name.starts_with("data/c/tags/");
    if shared_tag && value.get("replace").and_then(Value::as_bool) == Some(true) {
        return Some("replaces a shared tag; a content mod may only add to it".into());
    }
    None
}

/// Carries `jar` over into `out/carry/`. With the oracle's `facts`, blocks
/// and items are the ones the mod registers, with the templates and numbers
/// the facts give; `allowed` says which vanilla blocks the game has.
pub fn carry(
    jar: &Path,
    info: &JarInfo,
    out: &Path,
    facts: Option<&Facts>,
    allowed: &dyn Fn(&str) -> bool,
    conventions: &BTreeMap<String, Value>,
) -> Result<Report, String> {
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
    let mut convention = BTreeSet::new();
    let mut block_loot = BTreeSet::new();
    let mut biome_conventions = BTreeSet::new();
    let set_tags = crate::worldgen::structure_set_tags(&mut zip);
    let piece_counts = crate::worldgen::piece_counts(&mut zip);
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
        let unreadable = if keep {
            unreadable(&name, &mut bytes)
        } else {
            None
        };
        let keep = keep && unreadable.is_none();
        if let Some(why) = unreadable {
            report.unreadable.insert(name.clone(), why);
        }
        let dest = if keep {
            report.copied += 1;
            inside.join(&name)
        } else {
            report.outside.push(name.clone());
            outside.join(&name)
        };
        if keep {
            let rel = name.split_once(&format!("/{id}/")).map_or("", |(_, r)| r);
            if name.starts_with("data/")
                && (rel.starts_with("worldgen/") || rel.starts_with("structure/"))
            {
                report.worldgen += 1;
            }
            if let Some(file) = rel
                .strip_prefix("recipe/")
                .filter(|_| name.starts_with("data/"))
            {
                if let Ok(mut recipe) = serde_json::from_slice::<Value>(&bytes) {
                    tags_used(&recipe, &mut convention);
                    if upgrade_recipe(&mut recipe) {
                        report.recipes_upgraded += 1;
                        bytes = serde_json::to_vec_pretty(&recipe).unwrap_or(bytes);
                    } else {
                        report.recipes_kept.push(file.to_owned());
                    }
                }
            } else if let Some(kind) = rel
                .strip_prefix("worldgen/")
                .and_then(|r| r.split_once('/'))
                .map(|(kind, _)| kind)
                .filter(|_| name.starts_with("data/") && name.ends_with(".json"))
            {
                if let Ok(mut v) = serde_json::from_slice::<Value>(&bytes) {
                    if kind == "structure" {
                        crate::worldgen::convention_tags(&v["biomes"], &mut biome_conventions);
                    }
                    let mut changed = crate::worldgen::upgrade(
                        kind,
                        &mut v,
                        &mut report.worldgen_types,
                        &set_tags,
                    );
                    if kind == "structure" {
                        let path = rel.trim_start_matches("worldgen/structure/");
                        let structure = format!("{id}:{}", path.trim_end_matches(".json"));
                        changed |= crate::worldgen::disables(&structure, &mut v);
                        changed |=
                            crate::worldgen::add_piece_counts(&structure, &mut v, &piece_counts);
                    }
                    if changed {
                        bytes = serde_json::to_vec_pretty(&v).unwrap_or(bytes);
                    }
                }
            } else if rel.starts_with("tags/worldgen/biome/") && name.starts_with("data/") {
                if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                    crate::worldgen::convention_tags(&v, &mut biome_conventions);
                }
            } else if rel.starts_with("loot_table/") && name.starts_with("data/") {
                if let Some(b) = rel
                    .strip_prefix("loot_table/blocks/")
                    .and_then(|b| b.strip_suffix(".json"))
                {
                    block_loot.insert(b.to_owned());
                }
                if let Ok(mut table) = serde_json::from_slice::<Value>(&bytes) {
                    let before = table.clone();
                    upgrade_loot(&mut table);
                    if table != before {
                        report.loot_tables_upgraded += 1;
                        bytes = serde_json::to_vec_pretty(&table).unwrap_or(bytes);
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
    // The loader's convention biome tags the structures and their tags name
    // (NeoForge's `c:is_plains`...): the game has no loader, so they come along.
    for tag in crate::worldgen::with_included(&biome_conventions, conventions) {
        let file = inside.join(format!("data/c/tags/worldgen/biome/{tag}.json"));
        if !file.exists() {
            write(
                &file,
                &serde_json::to_vec_pretty(&conventions[&tag]).unwrap_or_default(),
            )?;
            report.convention_biome_tags += 1;
        }
    }
    let decl = inside.join(format!("data/{id}/horizon"));
    let write_decl = |kind: &str, name: &str, v: &Value| {
        let text = serde_json::to_vec_pretty(v).unwrap_or_default();
        write(&decl.join(format!("{kind}/{name}.json")), &text)
    };
    if let Some(facts) = facts {
        report.oracle = true;
        let prefix = format!("{id}:");
        let mut declared = BTreeSet::new();
        for (full, block) in facts.blocks.range(prefix.clone()..) {
            let Some(b) = full.strip_prefix(&prefix) else {
                break;
            };
            let Some(choice) = facts.template(full, allowed) else {
                let props: Vec<&str> = block.properties.keys().map(String::as_str).collect();
                let why = format!(
                    "no vanilla {} with properties [{}]",
                    block.vanilla_class().rsplit('.').next().unwrap_or(""),
                    props.join(", ")
                );
                report.blocks_unknown.insert(b.to_owned(), why);
                continue;
            };
            let mut v = json!({"template": choice.template, "properties": choice.properties});
            if block.item == "minecraft:air" {
                v["item"] = json!(false);
            } else if block.item != *full {
                v["item"] = json!(false);
                report
                    .differences
                    .entry(b.to_owned())
                    .or_default()
                    .push(format!("its item is {}", block.item));
            } else {
                declared.insert(full.clone());
                let drop = (!block_loot.contains(b))
                    .then(|| block.drops_itself(full))
                    .flatten();
                if let Some(drop) = drop {
                    let mut entry = json!({"type": "minecraft:item", "name": full});
                    if let crate::oracle::SelfDrop::When(property, value) = drop {
                        entry["condition"] = json!({
                            "type": "minecraft:match_block",
                            "blocks": full,
                            "state": {property: value}
                        });
                    }
                    let table = json!({
                        "type": "minecraft:block",
                        "pools": [{
                            "condition": {"type": "minecraft:survives_explosion"},
                            "entries": [entry],
                            "rolls": 1
                        }],
                        "random_sequence": format!("{id}:blocks/{b}")
                    });
                    let file = inside.join(format!("data/{id}/loot_table/blocks/{b}.json"));
                    write(
                        &file,
                        &serde_json::to_vec_pretty(&table).unwrap_or_default(),
                    )?;
                    report.loot_tables_added += 1;
                }
            }
            write_decl("block", b, &v)?;
            report.blocks.insert(b.to_owned(), choice.template);
            if !choice.differences.is_empty() {
                report
                    .differences
                    .entry(b.to_owned())
                    .or_default()
                    .extend(choice.differences);
            }
        }
        for (full, item) in facts.items.range(prefix.clone()..) {
            let Some(name) = full.strip_prefix(&prefix) else {
                break;
            };
            if declared.contains(full) {
                continue;
            }
            if item
                .block
                .as_ref()
                .is_some_and(|b| facts.blocks.contains_key(b))
                && report.blocks_unknown.contains_key(name)
            {
                report
                    .items_unknown
                    .insert(name.to_owned(), "its block is left out".into());
                continue;
            }
            match facts.item(full) {
                Ok((v, differences)) => {
                    write_decl("item", name, &v)?;
                    report.items += 1;
                    if !differences.is_empty() {
                        report.differences.insert(name.to_owned(), differences);
                    }
                }
                Err(e) => {
                    report.items_unknown.insert(name.to_owned(), e);
                }
            }
        }
        // Convention tags are NeoForge's and the pack's: the recipes get the
        // members the oracle saw, those of vanilla and of this mod.
        for tag in &convention {
            let Some(path) = tag.strip_prefix("c:") else {
                continue;
            };
            let file = inside.join(format!("data/c/tags/item/{path}.json"));
            if file.exists() {
                continue;
            }
            if let Some(members) = facts.tag_members("item", tag, id) {
                let v = json!({"values": members});
                write(&file, &serde_json::to_vec_pretty(&v).unwrap_or_default())?;
                report.convention_tags += 1;
            }
        }
    } else {
        for (b, state) in &blockstates {
            match template(b, &properties(state)) {
                Some(t) => {
                    write_decl("block", b, &json!({"template": t}))?;
                    report.blocks.insert(b.clone(), t.to_owned());
                }
                None => {
                    let props: Vec<String> = properties(state).into_iter().collect();
                    let why = format!("no template for properties [{}]", props.join(", "));
                    report.blocks_unknown.insert(b.clone(), why);
                }
            }
        }
        for m in item_models.iter().filter(|m| !blockstates.contains_key(*m)) {
            write_decl("item", m, &json!({"template": "minecraft:stick"}))?;
            report.items += 1;
        }
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

/// The tags a recipe's ingredients name (`#ns:path` or `{"tag": ...}`).
fn tags_used(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) => {
            if let Some(t) = s.strip_prefix('#') {
                out.insert(t.to_owned());
            }
        }
        Value::Array(a) => a.iter().for_each(|x| tags_used(x, out)),
        Value::Object(o) => {
            for (k, x) in o {
                match (k.as_str(), x) {
                    ("tag", Value::String(t)) => {
                        out.insert(t.clone());
                    }
                    ("result", _) => {}
                    _ => tags_used(x, out),
                }
            }
        }
        _ => {}
    }
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
    fn upgrades_loot_tables() {
        let mut slab = json!({"type": "minecraft:block", "pools": [{"rolls": 1.0, "entries": [{
            "type": "minecraft:item", "name": "m:slab",
            "functions": [{"function": "minecraft:set_count", "count": 2.0, "add": false,
                "conditions": [{"condition": "minecraft:block_state_property", "block": "m:slab",
                    "properties": {"type": "double"}}]},
                {"function": "minecraft:explosion_decay"}]}]}]});
        upgrade_loot(&mut slab);
        assert_eq!(
            slab["pools"][0]["entries"][0]["modifier"][0],
            json!({"type": "minecraft:set_count", "count": 2.0, "add": false,
                "condition": {"type": "minecraft:block_state_property", "block": "m:slab",
                    "properties": {"type": "double"}}})
        );
        let silk = json!({"condition": "minecraft:match_tool", "predicate": {"predicates": {
            "minecraft:enchantments": [{"enchantments": "minecraft:silk_touch", "levels": {"min": 1}}]}}});
        let shears = json!({"condition": "minecraft:match_tool", "predicate": {"items": "minecraft:shears"}});
        let mut glass = json!({"pools": [{"conditions": [silk.clone()], "entries": []},
            {"conditions": [{"condition": "minecraft:any_of", "terms": [silk, shears]},
                {"condition": "minecraft:survives_explosion"}], "entries": []}]});
        upgrade_loot(&mut glass);
        assert_eq!(
            glass["pools"][0]["condition"],
            "minecraft:tool/can_silk_touch"
        );
        assert_eq!(
            glass["pools"][1]["condition"],
            json!({"type": "minecraft:all_of", "terms": [
                {"type": "minecraft:any_of", "terms": ["minecraft:tool/can_silk_touch", "minecraft:tool/can_shear"]},
                {"type": "minecraft:survives_explosion"}]})
        );
        // A loot table already in 26.3's shape stays as it is.
        let mut new = glass.clone();
        upgrade_loot(&mut new);
        assert_eq!(new, glass);
    }

    #[test]
    fn templates_from_blockstate_properties() {
        let door =
            json!({"variants": {"facing=east,half=lower,hinge=left,open=false,powered=false": {}}});
        assert_eq!(
            template("gem_door", &properties(&door)),
            Some("minecraft:oak_door")
        );
        // Models that ignore `powered`: still a door.
        let door = json!({"variants": {"facing=east,half=lower,hinge=left,open=false": {}}});
        assert_eq!(
            template("gem_door", &properties(&door)),
            Some("minecraft:oak_door")
        );
        let trapdoor = json!({"variants": {"facing=east,half=top,open=false": {}}});
        assert_eq!(
            template("gem_trapdoor", &properties(&trapdoor)),
            Some("minecraft:oak_trapdoor")
        );
        let sides = json!({"multipart": [{"when": {"north": "true"}}, {"when": {"east": "true"}},
            {"when": {"south": "true"}}, {"when": {"west": "true"}}]});
        assert_eq!(
            template("gem_pane", &properties(&sides)),
            Some("minecraft:glass_pane")
        );
        assert_eq!(
            template("gem_fence", &properties(&sides)),
            Some("minecraft:oak_fence")
        );
        let fence = json!({"multipart": [{"apply": {}}, {"when": {"north": "true"}, "apply": {}},
            {"when": {"east": "true"}}, {"when": {"south": "true"}}, {"when": {"west": "true"}},
            {"when": {"OR": [{"waterlogged": "true"}]}}]});
        assert_eq!(
            template("gem_fence", &properties(&fence)),
            Some("minecraft:oak_fence")
        );
        assert_eq!(
            template("gem_block", &properties(&json!({"variants": {"": {}}}))),
            Some("minecraft:stone")
        );
        let odd = json!({"variants": {"charge=1,mode=fast": {}}});
        assert_eq!(template("gem_core", &properties(&odd)), None);
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
            // What the game would refuse: vanilla skips these files alone.
            ("assets/gem/lang/ry_UA.json", r#"{"item.gem.gem": "Gem"}"#),
            ("data/gem/tags/item/empty.json", ""),
            (
                "data/minecraft/tags/worldgen/biome/has_structure/village_plains.json",
                r#"{"replace": true, "values": []}"#,
            ),
            (
                "assets/gem/lang/de_de.json",
                "\u{feff}{\"item.gem.gem\": \"Juwel\"}",
            ),
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
        let report = carry(&jar, &info, &out, None, &|_| true, &BTreeMap::new()).unwrap();
        assert_eq!(report.copied, 13);
        let unreadable: Vec<&str> = report.unreadable.keys().map(String::as_str).collect();
        assert_eq!(
            unreadable,
            [
                "assets/gem/lang/ry_UA.json",
                "data/gem/tags/item/empty.json",
                "data/minecraft/tags/worldgen/biome/has_structure/village_plains.json",
            ]
        );
        assert_eq!(report.outside.len(), 3);
        assert!(!out.join("carry/data/gem/tags/item/empty.json").exists());
        // A byte-order mark is dropped, the file kept.
        assert_eq!(
            fs::read(out.join("carry/assets/gem/lang/de_de.json")).unwrap()[0],
            b'{'
        );
        assert_eq!((report.recipes_upgraded, report.recipes_kept.len()), (1, 1));
        assert_eq!(report.item_definitions_added, 2);
        assert_eq!(report.blocks["gem_door"], "minecraft:oak_door");
        assert_eq!(report.blocks["gem_block"], "minecraft:stone");
        assert_eq!(
            report.blocks_unknown.keys().collect::<Vec<_>>(),
            ["gem_core"]
        );
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
