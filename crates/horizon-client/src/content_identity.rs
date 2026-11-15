//! Identity of the ordered block-state registry used by chunk packets, and of
//! the item, mob effect, fluid, entity type, menu and custom statistic registries
//! once mods add to them. Their packets carry numeric network IDs.
use horizon_protocol::registry::Builtin;
use serde::{Deserialize, Serialize, de::IgnoredAny};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fmt, io,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub const CHANNEL: &str = "horizon:block_registry";
/// Decode the optional identity challenge; an empty body requests our digest.
/// Fixed-size validation happens before any owned decoding allocation.
pub fn decode_payload(data: &[u8]) -> horizon_protocol::Result<Option<[u8; 32]>> {
    if data.is_empty() {
        return Ok(None);
    }
    let digest = data
        .try_into()
        .map_err(|_| horizon_protocol::Error::new("block registry identity must be 32 bytes"))?;
    Ok(Some(digest))
}

pub const MARKER: &str = "horizon-content-blocks";
pub const MISMATCH: &str =
    "Content registry mismatch: install the same content mods as the server.";

/// Registries that a vanilla client cannot validate before reading their IDs.
pub(crate) fn requires_horizon() -> bool {
    horizon_protocol::item::custom::entries().next().is_some()
        || !mod_api::__private::custom_stat_declarations().is_empty()
}

#[derive(Debug)]
pub enum IdentityError {
    Read(io::Error),
    Json(serde_json::Error),
    Registries(String),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(f, "cannot read facts.json: {error}"),
            Self::Json(error) => write!(f, "invalid facts.json: {error}"),
            Self::Registries(error) => write!(f, "invalid registries.json: {error}"),
        }
    }
}

impl std::error::Error for IdentityError {}

#[derive(Deserialize)]
struct Facts {
    states: Vec<StateRow>,
}

#[derive(Deserialize)]
struct StateRow(IgnoredAny, IgnoredAny, IgnoredAny, IgnoredAny, State);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    properties: Option<BTreeMap<String, String>>,
}

struct CachedIdentity {
    generation: u64,
    hash: [u8; 32],
}

fn hash_block_entity_validity(
    hash: [u8; 32],
    types: &crate::block_entity_types::BlockEntityTypes,
) -> [u8; 32] {
    let entries: Vec<_> = types
        .identity_memberships()
        .enumerate()
        .map(|(index, (block, kind))| (index as u64, format!("{block}\0{kind}")))
        .collect();
    if entries.is_empty() {
        hash
    } else {
        hash_entries(hash, "horizon:block_entity_validity/v1", &entries)
    }
}

/// Hash the network order and each state's name and properties, not its gameplay facts.
pub fn identity(
    root: &Path,
    item_defaults: &horizon_world::item_defaults::ItemDefaults,
) -> Result<[u8; 32], IdentityError> {
    // Keep each resource root stable and avoid reparsing its large facts file for every join.
    static IDENTITIES: OnceLock<Mutex<HashMap<PathBuf, CachedIdentity>>> = OnceLock::new();
    let mut identities = IDENTITIES.get_or_init(Default::default).lock().unwrap();
    if let Some(cached) = identities.get(root)
        && cached.generation == item_defaults.generation()
    {
        return Ok(cached.hash);
    }
    let bytes = std::fs::read(root.join("facts.json")).map_err(IdentityError::Read)?;
    let mut hash = hash_facts(&bytes)?;
    // A root without mod entries keeps its block-only identity.
    let report = match std::fs::read(root.join("registries.json")) {
        Ok(bytes) => Some(crate::lan::data::with_custom_stats(parse_registries(
            &bytes,
        )?)),
        Err(error) if !mod_api::__private::custom_stat_declarations().is_empty() => {
            return Err(IdentityError::Read(error));
        }
        Err(_) => None,
    };
    if let Some(report) = report {
        for registry in REGISTRIES {
            let entries = mod_entries(&report, registry);
            if !entries.is_empty() {
                hash = hash_entries(hash, registry, &entries);
            }
        }
    }
    let membership = match std::fs::read(root.join(crate::mod_content::BLOCK_ENTITY_TYPES)) {
        Ok(bytes) => serde_json::from_slice::<BTreeMap<String, String>>(&bytes)
            .map_err(IdentityError::Json)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
        Err(error) => return Err(IdentityError::Read(error)),
    };
    if !membership.is_empty() {
        let entries: Vec<_> = membership
            .into_iter()
            .enumerate()
            .map(|(index, (block, kind))| (index as u64, format!("{block}\0{kind}")))
            .collect();
        hash = hash_entries(hash, "horizon:block_entity_membership/v1", &entries);
    }
    if let Some(types) = crate::block_entity_types::current() {
        hash = hash_block_entity_validity(hash, types);
    }
    let components: Vec<_> = horizon_protocol::item::custom::entries()
        .map(|(id, name)| (id as u64, name.to_owned()))
        .collect();
    if !components.is_empty() {
        hash = hash_entries(hash, "horizon:nbt_components/v1", &components);
        // Defaults are not sent in a stack patch. Both peers must therefore
        // agree on their values as well as the registered wire IDs, native
        // mods' changes included.
        let mut defaults = Vec::new();
        for (item, components) in item_defaults.entries() {
            for (name, value) in components.as_object().into_iter().flatten() {
                if horizon_protocol::item::custom::id(name).is_some() {
                    defaults.push((defaults.len() as u64, format!("{item}\0{name}\0{value}")));
                }
            }
        }
        if !defaults.is_empty() {
            hash = hash_entries(hash, "horizon:nbt_component_defaults/v1", &defaults);
        }
    }
    identities.insert(
        root.to_owned(),
        CachedIdentity {
            generation: item_defaults.generation(),
            hash,
        },
    );
    Ok(hash)
}

fn hash_facts(bytes: &[u8]) -> Result<[u8; 32], IdentityError> {
    let facts: Facts = serde_json::from_slice(bytes).map_err(IdentityError::Json)?;
    let mut hash = Sha256::new();
    for row in facts.states {
        let encoded = serde_json::to_vec(&row.4).map_err(IdentityError::Json)?;
        hash.update((encoded.len() as u64).to_le_bytes());
        hash.update(encoded);
    }
    Ok(hash.finalize().into())
}

/// The built-in registries whose network ids content mods extend besides blocks.
const REGISTRIES: [&str; 7] = [
    "minecraft:item",
    "minecraft:mob_effect",
    "minecraft:fluid",
    "minecraft:entity_type",
    "minecraft:menu",
    "minecraft:block_entity_type",
    "minecraft:custom_stat",
];

fn parse_registries(bytes: &[u8]) -> Result<Builtin, IdentityError> {
    let text = std::str::from_utf8(bytes).map_err(|e| IdentityError::Registries(e.to_string()))?;
    Builtin::parse(text).map_err(|e| IdentityError::Registries(e.to_string()))
}

/// The content mods' entries of `registry` in `registries.json`, by network id.
fn mod_entries(report: &Builtin, registry: &str) -> Vec<(u64, String)> {
    // Names come indexed by network id, so they are already in order.
    report
        .get(registry)
        .into_iter()
        .flat_map(|ids| ids.names.iter().enumerate())
        .filter(|(_, name)| !name.is_empty() && !name.starts_with("minecraft:"))
        .map(|(id, name)| (id as u64, name.clone()))
        .collect()
}

/// The identity so far extended with the mods' entries of one registry.
fn hash_entries(previous: [u8; 32], registry: &str, entries: &[(u64, String)]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(previous);
    hash.update((registry.len() as u64).to_le_bytes());
    hash.update(registry);
    for (id, name) in entries {
        hash.update(id.to_le_bytes());
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
    }
    hash.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::{hash_entries, hash_facts, identity, mod_entries, parse_registries};

    #[test]
    fn mod_items_and_effects_join_the_identity() {
        const EFFECT: &str = "minecraft:mob_effect";
        const ITEM: &str = "minecraft:item";
        let entries = |extra: &str, registry: &str| {
            let report = format!(
                r#"{{"minecraft:mob_effect":{{"entries":{{"minecraft:speed":{{"protocol_id":0}}{extra}}}}},
                "minecraft:item":{{"entries":{{"minecraft:air":{{"protocol_id":0}},"ruby:ruby":{{"protocol_id":1}}}}}}}}"#
            );
            mod_entries(&parse_registries(report.as_bytes()).unwrap(), registry)
        };
        assert!(entries("", EFFECT).is_empty());
        let ruby = entries("", ITEM);
        assert_eq!(ruby, [(1, "ruby:ruby".to_owned())]);
        let glow = entries(r#","ruby:glow":{"protocol_id":40}"#, EFFECT);
        assert_eq!(glow, [(40, "ruby:glow".to_owned())]);
        let zap = entries(r#","ruby:zap":{"protocol_id":40}"#, EFFECT);
        let blocks = [7; 32];
        assert_ne!(
            hash_entries(blocks, EFFECT, &glow),
            hash_entries(blocks, EFFECT, &zap)
        );
        assert_ne!(hash_entries(blocks, EFFECT, &glow), blocks);
        assert_ne!(hash_entries(blocks, ITEM, &ruby), blocks);
        // the same id and name in another registry is other content.
        assert_ne!(
            hash_entries(blocks, ITEM, &glow),
            hash_entries(blocks, EFFECT, &glow)
        );
    }

    /// a mod menu type in the report changes the identity, so a client
    /// without it is refused before it could be sent an id it lacks.
    #[test]
    fn a_mod_menu_type_joins_the_identity() {
        let root = |name: &str, extra: &str| {
            let root = std::env::temp_dir().join(format!(
                "horizon-content-identity-menu-{name}-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let facts = br#"{"states":[["minecraft:stone",false,false,"Block{minecraft:stone}",{"id":"minecraft:stone"}]]}"#;
            std::fs::write(root.join("facts.json"), facts).unwrap();
            let report = format!(
                r#"{{"minecraft:menu":{{"entries":{{"minecraft:generic_9x1":{{"protocol_id":0}}{extra}}}}}}}"#
            );
            std::fs::write(root.join("registries.json"), report).unwrap();
            std::fs::write(root.join("items.json"), "{}").unwrap();
            let defaults = horizon_world::item_defaults::read(&root).unwrap();
            let hash = identity(&root, &defaults).unwrap();
            std::fs::remove_dir_all(&root).unwrap();
            hash
        };
        let vanilla = root("vanilla", "");
        let press = root("press", r#","ruby:press":{"protocol_id":25}"#);
        assert_ne!(press, vanilla);
        assert_ne!(press, root("mill", r#","ruby:mill":{"protocol_id":25}"#));
    }

    /// a mod fluid in the report changes the identity: fluid tags are sent by
    /// fluid id, and a client without the fluid would read another's.
    #[test]
    fn a_mod_fluid_joins_the_identity() {
        let root = |name: &str, extra: &str| {
            let root = std::env::temp_dir().join(format!(
                "horizon-content-identity-fluid-{name}-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let facts = br#"{"states":[["minecraft:stone",false,false,"Block{minecraft:stone}",{"id":"minecraft:stone"}]]}"#;
            std::fs::write(root.join("facts.json"), facts).unwrap();
            let report = format!(
                r#"{{"minecraft:fluid":{{"entries":{{"minecraft:water":{{"protocol_id":2}}{extra}}}}}}}"#
            );
            std::fs::write(root.join("registries.json"), report).unwrap();
            std::fs::write(root.join("items.json"), "{}").unwrap();
            let defaults = horizon_world::item_defaults::read(&root).unwrap();
            let hash = identity(&root, &defaults).unwrap();
            std::fs::remove_dir_all(&root).unwrap();
            (hash, hash_facts(facts).unwrap())
        };
        let (vanilla, blocks) = root("vanilla", "");
        assert_eq!(vanilla, blocks, "vanilla's fluids are not mod content");
        let (honey, _) = root("honey", r#","ruby:honey":{"protocol_id":5}"#);
        assert_ne!(honey, vanilla);
        assert_ne!(honey, root("oil", r#","ruby:oil":{"protocol_id":5}"#).0);
    }

    #[test]
    fn block_entity_ids_and_membership_affect_identity() {
        let root = std::env::temp_dir().join(format!(
            "horizon-content-identity-block-entities-{}",
            std::process::id()
        ));
        let build = |label: &str, kind: &str, id: u32, membership: &str| {
            let root = root.join(label);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join("facts.json"),
                br#"{"states":[["test:press",false,false,"Block{test:press}",{"id":"test:press"}]]}"#,
            )
            .unwrap();
            std::fs::write(
                root.join("registries.json"),
                format!(
                    r#"{{"minecraft:block_entity_type":{{"entries":{{"minecraft:chest":{{"protocol_id":1}},"{kind}":{{"protocol_id":{id}}}}}}}}}"#
                ),
            )
            .unwrap();
            std::fs::write(
                root.join(crate::mod_content::BLOCK_ENTITY_TYPES),
                membership,
            )
            .unwrap();
            std::fs::write(root.join("items.json"), "{}").unwrap();
            let defaults = horizon_world::item_defaults::read(&root).unwrap();
            (identity(&root, &defaults).unwrap(), defaults)
        };
        let mapping = r#"{"test:press":"test:machine","test:mill":"test:machine"}"#;
        let (machine, defaults) = build("first", "test:machine", 44, mapping);
        assert_eq!(identity(&root.join("first"), &defaults).unwrap(), machine);
        assert_ne!(machine, build("wire", "test:machine", 45, mapping).0);
        assert_ne!(machine, build("name", "test:other", 44, mapping).0);
        assert_ne!(
            machine,
            build(
                "membership",
                "test:machine",
                44,
                r#"{"test:press":"test:machine"}"#
            )
            .0
        );
        assert_eq!(
            machine,
            build(
                "order",
                "test:machine",
                44,
                r#"{"test:mill":"test:machine","test:press":"test:machine"}"#
            )
            .0
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_root_without_mod_entries_keeps_its_block_identity() {
        let root = std::env::temp_dir().join(format!(
            "horizon-content-identity-vanilla-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let facts = br#"{"states":[["minecraft:stone",false,false,"Block{minecraft:stone}",{"id":"minecraft:stone"}]]}"#;
        std::fs::write(root.join("facts.json"), facts).unwrap();
        std::fs::write(
            root.join("registries.json"),
            br#"{"minecraft:item":{"entries":{"minecraft:air":{"protocol_id":0}}},
            "minecraft:mob_effect":{"entries":{"minecraft:speed":{"protocol_id":0}}}}"#,
        )
        .unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        assert_eq!(
            identity(&root, &defaults).unwrap(),
            hash_facts(facts).unwrap()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn identity_tracks_order_and_properties_not_gameplay_data() {
        let facts = |a: &str, b: &str| {
            format!(r#"{{"states":[["x",false,false,"x",{a}],["y",false,false,"y",{b}]]}}"#)
        };
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
            hash_facts(format!(r#"{{"states":[["x",false,false,"x",{a}]]}}"#).as_bytes()).unwrap()
        );
        assert!(hash_facts(br#"{"states":[["x",false,false,"x","minecraft:stone"]]}"#).is_err());
        assert!(hash_facts(br#"{"states":[[]]}"#).is_err());
        assert!(hash_facts(br#"{"states":[["x",false,false,"x",{"id":"x"},0]]}"#).is_err());
    }
}

#[cfg(test)]
mod validity_tests {
    use super::*;
    #[test]
    fn added_validity_changes_identity_without_changing_factories() {
        use crate::block_entity_types::BlockEntityTypes;
        let mut types = BlockEntityTypes::new(
            BTreeMap::new(),
            vec!["minecraft:chest".into(), "minecraft:trapped_chest".into()],
        );
        types.template(
            "minecraft:trapped_chest".into(),
            "minecraft:trapped_chest".into(),
            "minecraft:trapped_chest".into(),
        );
        assert_eq!(hash_block_entity_validity([42; 32], &types), [42; 32]);
        types
            .add_valid("minecraft:trapped_chest", "minecraft:chest")
            .unwrap();
        let added = hash_block_entity_validity([42; 32], &types);
        assert_ne!(added, [42; 32]);
        types
            .add_valid("minecraft:trapped_chest", "minecraft:chest")
            .unwrap();
        assert_eq!(hash_block_entity_validity([42; 32], &types), added);
        assert_eq!(
            types.template_for("minecraft:trapped_chest"),
            Some("minecraft:trapped_chest")
        );
    }
}

#[cfg(test)]
#[test]
fn custom_statistic_network_ids_join_identity() {
    let hash = |label: &str, entries: &str| {
        let root = std::env::temp_dir().join(format!(
            "horizon-content-identity-stats-{}-{label}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("facts.json"),
            br#"{"states":[["minecraft:stone",false,false,"Block{minecraft:stone}",{"id":"minecraft:stone"}]]}"#,
        ).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        std::fs::write(
            root.join("registries.json"),
            format!(r#"{{"minecraft:custom_stat":{{"entries":{{"minecraft:jump":{{"protocol_id":0}}{entries}}}}}}}"#),
        ).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let digest = identity(&root, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        digest
    };
    let shared = r#","stats:steps":{"protocol_id":1},"stats:other":{"protocol_id":2}"#;
    let server = hash("server", shared);
    assert_eq!(server, hash("matching-client", shared));
    assert_ne!(server, hash("missing-client", ""));
    assert_ne!(
        server,
        hash(
            "reordered-client",
            r#","stats:other":{"protocol_id":1},"stats:steps":{"protocol_id":2}"#,
        )
    );
    assert_ne!(
        server,
        hash(
            "client-only-first",
            r#","client:local":{"protocol_id":1},"stats:steps":{"protocol_id":2},"stats:other":{"protocol_id":3}"#,
        )
    );
}
