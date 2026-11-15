use super::*;
/// Select one JSON resource per ID before applying the owning load's conditions.
/// A skipped winning resource stays absent; it never uncovers a lower version.
pub(crate) fn data_files(root: &Path, kind: &str, overlay: &PackOverlay) -> HashMap<String, Value> {
    let mut files: HashMap<_, _> = horizon_world::damage_source::resource_files(root, kind)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, bytes)| {
            let value = if kind == "enchantment" {
                let path = root
                    .join("data")
                    .join(id.replace(':', "/enchantment/") + ".json");
                horizon_world::json_folder::enchantment_definition(&bytes, &path)
            } else {
                serde_json::from_slice::<Value>(&bytes).ok()
            };
            value.map(|v| (id, v))
        })
        .collect();
    for (id, versions) in overlay.kind(kind) {
        if let Ok(value) = serde_json::from_slice::<Value>(PackOverlay::top(versions)) {
            files.insert(id.to_owned(), value);
        }
    }
    files.retain(|id, value| overlay.conditions.allows_id(value, kind, id));
    files
}
