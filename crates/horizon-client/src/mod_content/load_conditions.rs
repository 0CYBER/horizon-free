//! Content-root condition selection through the owning reload snapshot.
use super::*;

/// Evaluate a tag layer before its values or replace flag participate in a merge.
pub(super) fn tag_conditions_hold(
    doc: &J,
    conditions: &horizon_world::conditions::LoadConditions,
    path: &str,
) -> Result<bool> {
    if doc.get("neoforge:conditions").is_none() {
        return Ok(true);
    }
    let allowed = conditions.allows_file(&serde_json::to_value(doc)?, path);
    conditions.check_errors()?;
    Ok(allowed)
}

/// A tag file's `key` list (`values`, `remove`); empty where it has none.
pub(super) fn tag_list<'a>(tag: &'a J, key: &str) -> &'a [J] {
    match tag.get(key) {
        Some(J::Arr(list)) => list,
        _ => &[],
    }
}

/// A tag file's accepted values with `more` appended (no repeats), and
/// `remove` after its own `remove` entries (NeoForge's tag `remove` list).
/// The mods' layers of one tag, in load order: each layer's `values` and,
/// after them, its `remove` entries (`TagLoader`: a file adds, then removes).
pub(super) struct TagLayers<'a> {
    pub(super) values: &'a [J],
    pub(super) remove: &'a [J],
}

/// `vanilla`'s tag file over what the mods made of it: the mods' layers join
/// its `values` as ordered entries, a removal carried as `{"id": ..., "remove":
/// true}` (`TagLoader.EntryWithSource.remove`), so the folder walks adds and
/// removals in the mods' load order instead of all adds before all removals.
pub(super) fn merge_tag(
    text: Option<&str>,
    layers: &[TagLayers<'_>],
    what: &str,
    _conditions: &horizon_world::conditions::LoadConditions,
) -> Result<String> {
    let mut doc = match text {
        Some(t) => serde_json::from_str(t).map_err(|e| format!("{what}: {e}"))?,
        None => J::Obj(vec![("values".into(), J::Arr(Vec::new()))]),
    };
    // Tags do not evaluate load conditions.
    doc.remove("neoforge:conditions");
    // The vanilla file's own `remove` entries apply right after its values,
    // before the mods' layers: they join `values` as removal objects.
    let vanilla_remove = match doc.get("remove") {
        Some(J::Arr(remove)) => Some(remove.clone()),
        _ => None,
    };
    doc.remove("remove");
    let Some(J::Arr(values)) = doc.get_mut("values") else {
        return Err(format!("{what}: no values").into());
    };
    if let Some(remove) = vanilla_remove {
        for r in &remove {
            let removal = match r {
                J::Str(id) => J::Obj(vec![
                    ("id".into(), J::Str(id.clone())),
                    ("remove".into(), J::Bool(true)),
                ]),
                other => {
                    let mut entry = other.clone();
                    entry.set("remove", J::Bool(true));
                    entry
                }
            };
            values.push(removal);
        }
    }
    for layer in layers {
        for v in layer.values {
            values.push(v.clone());
        }
        for r in layer.remove {
            let removal = match r {
                J::Str(id) => J::Obj(vec![
                    ("id".into(), J::Str(id.clone())),
                    ("remove".into(), J::Bool(true)),
                ]),
                other => {
                    let mut entry = other.clone();
                    entry.set("remove", J::Bool(true));
                    entry
                }
            };
            values.push(removal);
        }
    }
    doc.remove("remove");
    Ok(serde_json::to_string_pretty(&doc)?)
}

/// What the mods do to one tag file, in load order (a data pack's
/// TagLoader): each adds its values, and one that replaces the tag first
/// drops vanilla's entries and the mods' before it.
#[derive(Default)]
pub(super) struct TagMerge {
    /// A mod replaced the tag: vanilla's entries are left out.
    pub(super) replaced: bool,
    /// The mods whose values are in `values`.
    pub(super) mods: Vec<String>,
    /// Each mod file's additions and, after them, its removals, in load
    /// order (`TagLoader`: one file adds its values, then its `remove`
    /// entries apply). A replace drops the layers below it.
    pub(super) layers: Vec<(Vec<J>, Vec<J>)>,
}

impl TagMerge {
    /// Adds one mod's tag file. A replace that drops entries (vanilla's in a
    /// `shared` tag, one not in the mod's namespaces, or other mods') warns.
    pub(super) fn add(
        &mut self,
        id: &str,
        file: &str,
        replace: bool,
        shared: bool,
        values: &[J],
    ) -> Option<String> {
        let mut warning = None;
        if replace {
            let mut dropped: Vec<String> = Vec::new();
            if shared && !self.replaced {
                dropped.push("vanilla's entries".into());
            }
            if !self.mods.is_empty() {
                dropped.push(format!("those of {}", self.mods.join(", ")));
            }
            if !dropped.is_empty() {
                warning = Some(format!(
                    "mod {id} replaces {file}: {} are left out, as a data pack's replace \
                     does; the mods after it in load order still add to it",
                    dropped.join(" and ")
                ));
            }
            self.replaced = true;
            self.mods.clear();
            self.layers.clear();
        }
        if !self.mods.iter().any(|m| m == id) {
            self.mods.push(id.to_owned());
        }
        self.layers.push((values.to_vec(), Vec::new()));
        warning
    }

    /// The layers' entries for [`merge_tag`].
    pub(super) fn layers(&self) -> Vec<TagLayers<'_>> {
        self.layers
            .iter()
            .map(|(values, remove)| TagLayers { values, remove })
            .collect()
    }
}

/// Build generated tables using the owning load's registered condition snapshot.
/// Refuse invalid conditions before publishing the generated root.
pub fn build_with_conditions(
    base: &Path,
    jar_store: &Path,
    mods: &[ContentMod],
    out: &Path,
    inventory: &horizon_mod_package::enabled::EnabledMods,
    conditions: &horizon_world::conditions::LoadConditions,
) -> Result<Vec<String>> {
    // Two mods owning one namespace would mix their files.
    for (i, a) in mods.iter().enumerate() {
        for b in &mods[..i] {
            let (b, a) = (
                (b.id.as_str(), b.content.namespaces.as_slice()),
                (a.id.as_str(), a.content.namespaces.as_slice()),
            );
            if let Some(e) = horizon_mod_package::content::clash(b, a) {
                return Err(e.into());
            }
        }
    }
    // The overlay links to the base: a relative base would leave them dangling.
    let base = &std::path::absolute(base)?;
    let loaded = load_layers(jar_store, mods)?;
    let plan = Plan::new(base, mods)?;
    // Publish the completed tables as one generation. Passive asset layers
    // retain their original sources instead of being written into this tree.
    let tables_root = out.with_file_name(format!(
        "{}-tables",
        out.file_name().unwrap_or_default().to_string_lossy()
    ));
    let _staging = super::root_lifetime::StagingRoot(tables_root.clone());
    let overlay = Overlay::new(base, &tables_root)?;
    let jobs = table_jobs(base, &overlay)?;
    tables(base, &plan, &overlay, &jobs)?;
    // Mod files, and every tag addition gathered per tag file.
    let mut tags: BTreeMap<String, TagMerge> = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut worldgen: BTreeMap<&str, Vec<(String, String)>> = BTreeMap::new();
    let mut additions: BTreeMap<&str, Vec<Addition>> = BTreeMap::new();
    let mut modifiers = Vec::new();
    let mut structure_modifiers = Vec::new();
    let mut namespaces: std::collections::BTreeSet<String> = mods
        .iter()
        .flat_map(|m| m.content.namespaces.iter().cloned())
        .collect();
    for group in &loaded {
        let m = group.source;
        let declarations = format!("data/{}/horizon/", m.id);
        for layer in &group.layers {
            if let Some(jar) = layer.jar {
                for name in layer.data.list("assets/").into_iter().filter(|name| {
                    !crate::packs::merge_stacks(name) && m.content.files.contains(name)
                }) {
                    warnings.push(format!(
                        "mod {}: jar {} also provides {name}; the mod's own file wins",
                        m.id,
                        jar.id()
                    ));
                }
            }
            for file in layer.data.list("data/") {
                if let Some((namespace, _)) = file
                    .strip_prefix("data/")
                    .and_then(|rest| rest.split_once('/'))
                {
                    namespaces.insert(namespace.to_owned());
                }
                let is_tag = file.contains("/tags/") && file.ends_with(".json");
                if !is_tag
                    && (file.starts_with(&declarations)
                        || (!biomes::is_modifier(&file)
                            && !structures::is_modifier(&file)
                            && worldgen_entry(&file).is_none()))
                {
                    continue;
                }
                let bytes = layer.data.read(&file).ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("{}: {file}", layer.assets.id),
                    )
                })?;
                // Dynamic registry entries use the same owning-load conditions as tags.
                // Evaluate before parsing required modifier fields.
                if biomes::is_modifier(&file) || structures::is_modifier(&file) {
                    let doc: J = serde_json::from_slice(&bytes)
                        .map_err(|e| format!("mod {}: {file}: {e}", m.id))?;
                    if !tag_conditions_hold(&doc, conditions, &file)? {
                        continue;
                    }
                }
                if is_tag {
                    let doc: J =
                        serde_json::from_slice(&bytes).map_err(|e| format!("{file}: {e}"))?;
                    let Some(J::Arr(values)) = doc.get("values") else {
                        return Err(format!("{}: {file}: no values", m.id).into());
                    };
                    let replace = doc.get("replace") == Some(&J::Bool(true));
                    let ns = file.split('/').nth(1).unwrap_or_default();
                    let shared = !m.content.namespaces.iter().any(|n| n == ns);
                    let merge = tags.entry(file.clone()).or_default();
                    warnings.extend(merge.add(&m.id, &file, replace, shared, values));
                    if let Some((_, remove)) = merge.layers.last_mut() {
                        remove.extend(tag_list(&doc, "remove").iter().cloned());
                    }
                } else if !file.starts_with(&declarations) {
                    if structures::is_modifier(&file) {
                        let text = String::from_utf8(bytes).map_err(|e| format!("{file}: {e}"))?;
                        structure_modifiers.extend(structures::Modifier::read(
                            &m.id,
                            &file,
                            &text,
                            &mut warnings,
                        )?);
                    } else if biomes::is_modifier(&file) {
                        let text = String::from_utf8(bytes).map_err(|e| format!("{file}: {e}"))?;
                        modifiers.extend(biomes::Modifier::read(
                            &m.id,
                            &file,
                            &text,
                            &mut warnings,
                        )?);
                    } else if let Some((bundle, id)) = worldgen_entry(&file) {
                        let text = String::from_utf8(bytes).map_err(|e| format!("{file}: {e}"))?;
                        let entries = worldgen.entry(bundle).or_default();
                        match Addition::read(bundle, &text).map_err(|e| format!("{file}: {e}"))? {
                            Some(addition) => additions.entry(bundle).or_default().push(addition),
                            None => entries.push((id, text)),
                        }
                    }
                }
            }
        }
    }
    // A tag naming an id nobody registers fails as a whole in vanilla, and
    // carried-over mods name blocks left out or other mods' ids: such
    // entries are dropped instead, so the rest of the tag holds.
    let known = Known::new(base, &plan, &tags)?;
    for (file, merge) in &mut tags {
        for (values, _) in &mut merge.layers {
            values.retain(|v| known.keeps(file, v));
        }
    }
    let mut bundles: Vec<(&str, J)> = Vec::new();
    for name in TAG_BUNDLES {
        if base.join(name).exists() {
            bundles.push((name, read_json(&base.join(name))?));
        }
    }
    for (file, merge) in &tags {
        let vanilla = fs::read_to_string(base.join(file))
            .ok()
            .filter(|_| !merge.replaced);
        overlay.write(
            file,
            merge_tag(vanilla.as_deref(), &merge.layers(), file, conditions)?.as_bytes(),
        )?;
        // data/<ns>/tags/block/<path>.json is the block tag <ns>:<path>.
        let Some((ns, path)) = file
            .strip_prefix("data/")
            .and_then(|r| r.split_once("/tags/block/"))
            .and_then(|(ns, p)| Some((ns, p.strip_suffix(".json")?)))
        else {
            continue;
        };
        let id = format!("{ns}:{path}");
        for (name, doc) in &mut bundles {
            let text = match doc.get(&id) {
                Some(J::Str(t)) if !merge.replaced => Some(t.clone()),
                _ => None,
            };
            let merged = merge_tag(text.as_deref(), &merge.layers(), name, conditions)?;
            doc.set(&id, J::Str(merged));
        }
    }
    for (name, doc) in &bundles {
        overlay.write(name, &serde_json::to_vec(doc)?)?;
    }
    // Biome tags, merged into the worldgen loader's bundle as block tags are.
    let biome_tags: Vec<(String, &TagMerge)> = tags
        .iter()
        .filter_map(|(file, merge)| {
            let (ns, path) = file
                .strip_prefix("data/")?
                .split_once("/tags/worldgen/biome/")?;
            Some((format!("{ns}:{}", path.strip_suffix(".json")?), merge))
        })
        .collect();
    let mut biome_tag_doc = None;
    if !biome_tags.is_empty() && base.join(BIOME_TAGS).exists() {
        let mut doc = read_json(&base.join(BIOME_TAGS))?;
        for (id, merge) in biome_tags {
            let text = match doc.get(&id) {
                Some(J::Str(t)) if !merge.replaced => Some(t.clone()),
                _ => None,
            };
            let merged = merge_tag(text.as_deref(), &merge.layers(), BIOME_TAGS, conditions)?;
            doc.set(&id, J::Str(merged));
        }
        overlay.write(BIOME_TAGS, &serde_json::to_vec(&doc)?)?;
        biome_tag_doc = Some(doc);
    }
    // Structures and the pools, sets, processors and features they use, then
    // what mods add to sets, pools and lists, by priority, then in mod and
    // file order.
    for list in additions.values_mut() {
        list.sort_by_key(|a| a.priority);
    }
    let mut placed_doc = None;
    for (bundle, entries) in &worldgen {
        if !base.join(bundle).exists() {
            continue;
        }
        let mut doc = read_json(&base.join(bundle))?;
        for (id, text) in entries {
            doc.set(id, J::Str(text.clone()));
        }
        for addition in additions.get(bundle).into_iter().flatten() {
            addition
                .apply(&mut doc)
                .map_err(|e| format!("{bundle}: {e}"))?;
        }
        overlay.write(bundle, &serde_json::to_vec(&doc)?)?;
        if *bundle == PLACED_FEATURES {
            placed_doc = Some(doc);
        }
    }
    if !modifiers.is_empty() || (!mods.is_empty() && base.join(BIOMES).exists()) {
        place_in_biomes(
            base,
            &overlay,
            &modifiers,
            inventory.server_ids().map(str::to_owned).collect(),
            biome_tag_doc,
            placed_doc,
            &tags,
            &mut warnings,
        )?;
    }
    if !structure_modifiers.is_empty() {
        structures::apply(base, &overlay, &structure_modifiers, &tags, &mut warnings)?;
    }
    // The commands suggest ids from `command-registries.json`: add the
    // structures, pools, sets and features the mods bring.
    let registries = out_path(&overlay, "command-registries.json");
    if let Ok(mut doc) = read_json(&registries) {
        // Registry names hold `/`, so each key is looked up by itself.
        let mut add = |keys: &[&str], new: &[J]| {
            let found = keys.iter().try_fold(&mut doc, |j, k| j.get_mut(k));
            if let Some(J::Arr(ids)) = found {
                for id in new {
                    if !ids.contains(id) {
                        ids.push(id.clone());
                    }
                }
            }
        };
        for (bundle, registry) in COMMAND_REGISTRIES {
            let new: Vec<J> = worldgen
                .get(bundle)
                .into_iter()
                .flatten()
                .map(|(id, _)| J::Str(id.clone()))
                .collect();
            add(&["by_registry", registry, "ids"], &new);
        }
        overlay.write("command-registries.json", &serde_json::to_vec(&doc)?)?;
    }
    // A block without its own blockstate looks like its template, so every
    // state still has a model.
    for m in mods {
        for block in &m.content.blocks {
            let (ns, path) = block.id.split_once(':').unwrap_or((&m.id, &block.id));
            let own = format!("assets/{ns}/blockstates/{path}.json");
            if m.content.files.contains(&own) {
                continue;
            }
            let template = block.template.trim_start_matches("minecraft:");
            let vanilla = base.join(format!("assets/minecraft/blockstates/{template}.json"));
            overlay.write(&own, &fs::read(&vanilla)?)?;
        }
    }
    // Vanilla's namespace isn't a content mod's, even when a mod ships files
    // under it (`data/minecraft/tags/...`): every loader reads it already, and
    // listed again it read each vanilla recipe twice, once as `minecraft:<name>`.
    namespaces.remove("minecraft");
    overlay.write(MARKER, &serde_json::to_vec(&namespaces)?)?;
    let requirements =
        collect_jar_requirements(mods.iter().map(|m| (m.id.as_str(), m.jars.as_slice())))?;
    overlay.write(JAR_INPUTS, &serde_json::to_vec(&requirements)?)?;
    overlay.write(
        horizon_mod_package::enabled::MARKER,
        &serde_json::to_vec(inventory)?,
    )?;
    let by_item = |value: fn(&NewItem) -> Option<&str>| {
        let items = plan.items.iter();
        items
            .filter_map(|i| Some((i.rename.to.as_str(), value(i)?)))
            .collect::<BTreeMap<_, _>>()
    };
    use horizon_items::items::{MOD_CRAFT_REMAINDERS, MOD_ITEM_BLOCKS, MOD_ITEM_TEMPLATES};
    for (name, map) in [
        (
            MOD_CRAFT_REMAINDERS,
            by_item(|i| i.crafting_remainder.as_deref()),
        ),
        (MOD_ITEM_TEMPLATES, by_item(|i| i.template.as_deref())),
        (MOD_ITEM_BLOCKS, by_item(|i| i.block.as_deref())),
    ] {
        if !map.is_empty() {
            overlay.write(name, &serde_json::to_vec(&map)?)?;
        }
    }
    let arm_poses: BTreeMap<&str, horizon_mod_package::content::ArmPose> = plan
        .items
        .iter()
        .filter_map(|i| Some((i.rename.to.as_str(), i.arm_pose?)))
        .collect();
    if !arm_poses.is_empty() {
        overlay.write(
            horizon_items::items::MOD_ITEM_ARM_POSES,
            &serde_json::to_vec(&arm_poses)?,
        )?;
    }
    overlay.write(
        BLOCK_ENTITY_TYPES,
        &serde_json::to_vec(&plan.block_entity_factories)?,
    )?;
    // clients must agree on the block states and the items', mob effects',
    // entity types', block entity types' and menu types' network ids.
    if !plan.blocks.is_empty()
        || !plan.items.is_empty()
        || !plan.effects.is_empty()
        || !plan.fluids.is_empty()
        || !plan.entity_types.is_empty()
        || !plan.block_entity_types.is_empty()
        || !plan.menu_types.is_empty()
    {
        overlay.write(crate::content_identity::MARKER, b"1")?;
    }
    overlay.write(WARNINGS, warnings.join("\n").as_bytes())?;
    let generated = serde_json::to_vec(
        &*overlay
            .written
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )?;
    overlay.write(GENERATED_FILES, &generated)?;
    conditions.check_errors()?;
    let _ = fs::remove_dir_all(out);
    fs::rename(&tables_root, out)?;
    Ok(warnings)
}

/// Builds a root using the owning load's condition registrations.
pub fn resource_root_with_conditions(
    base: &Path,
    jar_store: &Path,
    mods: &[ContentMod],
    cache: &Path,
    inventory: &horizon_mod_package::enabled::EnabledMods,
    conditions: &horizon_world::conditions::LoadConditions,
) -> Result<PathBuf> {
    let has_content = mods.iter().any(|m| {
        !m.jars.is_empty()
            || !m.content.is_empty()
            || !m.content.blocks.is_empty()
            || !m.content.items.is_empty()
            || !m.content.block_entity_types.is_empty()
    });
    if inventory.is_empty() && !has_content {
        return Ok(base.to_owned());
    }
    let mut stamp = stamp(base, jar_store, mods, inventory)?;
    if conditions.has_registered() {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        stamp.hash(&mut hash);
        conditions.hash(&mut hash);
        // Handlers can read config that has no persistent fingerprint. Give each
        // build a fresh root, leaving every previously usable root untouched.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
            .hash(&mut hash);
        stamp = format!("{:016x}{stamp}", hash.finish());
    }
    let roots = cache.join("mod-content");
    let out = roots.join(&stamp[..ROOT_NAME]);
    if fs::read(out.join(STAMP)).is_ok_and(|s| s == stamp.as_bytes())
        && fs::read(out.join(horizon_mod_package::enabled::MARKER)).is_ok_and(|bytes| {
            serde_json::from_slice::<horizon_mod_package::enabled::EnabledMods>(&bytes)
                .is_ok_and(|loaded| &loaded == inventory)
        })
    {
        // A reused root warns as the build did.
        let warnings = fs::read_to_string(out.join(WARNINGS)).unwrap_or_default();
        warnings.lines().for_each(|w| log::warn!("Horizon: {w}"));
        // Used again: kept another day. Marked beside the root, not in it: the pack overlay
        // laid over the root reads its files, and a rewritten one would rebuild it (#12403).
        let _ = fs::write(used_path(&out), b"");
        return Ok(out);
    }
    if has_content {
        for warning in build_with_conditions(base, jar_store, mods, &out, inventory, conditions)? {
            log::warn!("Horizon: {warning}");
        }
    } else {
        // Code-only packages still own an inventory generation, but need no
        // registry-table rewrite or content scan. Existing tables stay linked.
        let base = std::path::absolute(base)?;
        let overlay = Overlay::new(&base, &out)?;
        overlay.write(
            horizon_mod_package::enabled::MARKER,
            &serde_json::to_vec(inventory)?,
        )?;
        overlay.write(GENERATED_FILES, b"[]")?;
    }
    fs::write(out.join(STAMP), stamp)?;
    prune_roots(&roots, &out, KEEP_ROOTS);
    Ok(out)
}
