//! Identity of the ordered block-state registry used by chunk packets, and of
//! the mob effect registry once content mods add to it (effect packets carry
//! its network ids).
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub const CHANNEL: &str = "horizon:block_registry";
pub const MARKER: &str = "horizon-content-blocks";
pub const MISMATCH: &str =
    "Content registry mismatch: install the same content mods as the server.";

/// Hash the network order and each state's name and properties, not its gameplay facts.
pub fn identity(root: &Path) -> Result<[u8; 32], String> {
    static IDENTITIES: OnceLock<Mutex<HashMap<PathBuf, [u8; 32]>>> = OnceLock::new();
    let mut identities = IDENTITIES.get_or_init(Default::default).lock().unwrap();
    if let Some(hash) = identities.get(root) {
        return Ok(*hash);
    }
    let bytes = std::fs::read(root.join("facts.json")).map_err(|e| e.to_string())?;
    let mut hash = hash_facts(&bytes)?;
    let effects = match std::fs::read(root.join("registries.json")) {
        Ok(registries) => mod_effects(&registries)?,
        Err(_) => Vec::new(),
    };
    if !effects.is_empty() {
        hash = hash_effects(hash, &effects);
    }
    identities.insert(root.to_owned(), hash);
    Ok(hash)
}

fn hash_facts(bytes: &[u8]) -> Result<[u8; 32], String> {
    let facts: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let states = facts
        .get("states")
        .and_then(|s| s.as_array())
        .ok_or("facts.json has no block states")?;
    let mut hash = Sha256::new();
    for row in states {
        let state = row
            .as_array()
            .and_then(|r| r.get(4))
            .ok_or("facts.json has an invalid block state")?;
        let encoded = serde_json::to_vec(state).map_err(|e| e.to_string())?;
        hash.update((encoded.len() as u64).to_le_bytes());
        hash.update(encoded);
    }
    Ok(hash.finalize().into())
}

/// The content mods' mob effects in `registries.json`, by network id.
fn mod_effects(bytes: &[u8]) -> Result<Vec<(u64, String)>, String> {
    let report: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let mut effects: Vec<(u64, String)> = report["minecraft:mob_effect"]["entries"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(name, _)| !name.starts_with("minecraft:"))
        .filter_map(|(name, e)| Some((e["protocol_id"].as_u64()?, name.clone())))
        .collect();
    effects.sort();
    Ok(effects)
}

/// The block identity extended with the mods' effects; a root without them
/// keeps its block-only identity.
fn hash_effects(blocks: [u8; 32], effects: &[(u64, String)]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(blocks);
    for (id, name) in effects {
        hash.update(id.to_le_bytes());
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
    }
    hash.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::{hash_effects, hash_facts, mod_effects};

    #[test]
    fn mod_effects_join_the_identity() {
        let report = |extra: &str| {
            format!(
                r#"{{"minecraft:mob_effect":{{"entries":{{"minecraft:speed":{{"protocol_id":0}}{extra}}}}}}}"#
            )
        };
        assert!(mod_effects(report("").as_bytes()).unwrap().is_empty());
        let glow = mod_effects(report(r#","ruby:glow":{"protocol_id":40}"#).as_bytes()).unwrap();
        assert_eq!(glow, [(40, "ruby:glow".to_owned())]);
        let zap = mod_effects(report(r#","ruby:zap":{"protocol_id":40}"#).as_bytes()).unwrap();
        let blocks = [7; 32];
        assert_ne!(hash_effects(blocks, &glow), hash_effects(blocks, &zap));
        assert_ne!(hash_effects(blocks, &glow), blocks);
    }

    #[test]
    fn identity_tracks_order_and_properties_not_gameplay_data() {
        let facts = |a: &str, b: &str| format!(r#"{{"states":[["x",0,0,0,{a}],["y",0,0,0,{b}]]}}"#);
        let a = r#"{"id":"demo:a","properties":{"facing":"north"}}"#;
        let b = r#"{"id":"demo:b"}"#;
        let first = hash_facts(facts(a, b).as_bytes()).unwrap();
        assert_eq!(first, hash_facts(facts(a, b).as_bytes()).unwrap());
        assert_ne!(first, hash_facts(facts(b, a).as_bytes()).unwrap());
        assert_ne!(first, hash_facts(facts(a, a).as_bytes()).unwrap());
        assert_ne!(
            first,
            hash_facts(facts(&a.replace("north", "south"), b).as_bytes()).unwrap()
        );
        assert_ne!(
            first,
            hash_facts(format!(r#"{{"states":[["x",0,0,0,{a}]]}}"#).as_bytes()).unwrap()
        );
        assert!(hash_facts(br#"{"states":[[0,0,0,0,"minecraft:stone"]]}"#).is_ok());
        assert!(hash_facts(br#"{"states":[[]]}"#).is_err());
    }
}
