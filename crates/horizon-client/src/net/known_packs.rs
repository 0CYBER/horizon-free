//! The known packs of configuration: which of the server's packs the client names, and the
//! registry entries it then fills from its own copy of them.
use horizon_protocol::packets::configuration as cfg;
use std::path::Path;

/// `KnownPacksManager.trySelectingPacks`: the packs of the server's list the
/// client has too. Horizon carries vanilla's core pack (the prepared
/// resources' `data/`), so it names that one when the server offers it, and
/// the server leaves that pack's entries' data out (`fill_known_entries`).
pub(super) fn select_known_packs(offered: Vec<cfg::KnownPack>) -> Vec<cfg::KnownPack> {
    let (namespace, id, version) = crate::lan::data::CORE_PACK;
    offered
        .into_iter()
        .filter(|p| p.namespace == namespace && p.id == id && p.version == version)
        .collect()
}

fn valid_resource_segment(s: &str, allow_slash: bool) -> bool {
    !s.is_empty()
        && !s.starts_with('/')
        && !s.ends_with('/')
        && !s.contains("//")
        && s.split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && s.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'_' | b'-' | b'.')
                || (allow_slash && b == b'/')
        })
}

/// `RegistryDataCollector`: an entry of a pack both sides know comes without
/// its data, which the client reads from its own copy of the pack
/// (`data/<namespace>/<registry path>/<path>.json`, `Registries.elementsDirPath`).
pub(super) fn fill_known_entries(r: &mut cfg::RegistryData, resources: &Path) {
    let (registry_ns, registry_path) = r
        .registry
        .split_once(':')
        .unwrap_or(("minecraft", r.registry.as_str()));
    if !valid_resource_segment(registry_ns, false) || !valid_resource_segment(registry_path, true) {
        return;
    }
    let dir = if registry_ns == "minecraft" {
        registry_path.to_owned()
    } else {
        format!("{registry_ns}/{registry_path}")
    };
    for entry in r.entries.iter_mut().filter(|e| e.data.is_none()) {
        let (ns, path) = entry
            .id
            .split_once(':')
            .unwrap_or(("minecraft", entry.id.as_str()));
        if !valid_resource_segment(ns, false) || !valid_resource_segment(path, true) {
            continue;
        }
        let file = resources
            .join("data")
            .join(ns)
            .join(&dir)
            .join(format!("{path}.json"));
        let json = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        if let Some(json) = json {
            entry.data = Some(crate::lan::data::json_to_nbt(&json));
        }
    }
}
