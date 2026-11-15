use super::*;
/// A kind's overlay files over what the resource root provided: the top
/// pack's file for each id replaces the rest (`SimpleJsonResourceReloadListener`
/// sees one resource per id).
pub(super) fn overlay_map(overlay: &PackOverlay, kind: &str, out: &mut HashMap<String, Value>) {
    for (id, files) in overlay.kind(kind) {
        let Ok(v) = serde_json::from_slice::<Value>(PackOverlay::top(files)) else {
            continue;
        };
        out.insert(id.to_owned(), v);
    }
    out.retain(|id, value| overlay.conditions.allows_id(value, kind, id));
}
