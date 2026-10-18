//! The part of `facts.json` the block registry reads, deserialized as it
//! streams: each row's state (its fifth column) and the defaults.
use crate::chunk::BlockState;
use serde::{
    Deserialize, Deserializer,
    de::{self, IgnoredAny, SeqAccess, Visitor},
};
use std::{collections::BTreeMap, fmt};

#[derive(Deserialize)]
pub(super) struct Facts {
    pub states: Vec<Row>,
    pub defaults: BTreeMap<String, i64>,
}

/// One `states` row: `[name, air, fluid, description, state, ...]`, keeping
/// the two flags and the state.
pub(super) struct Row {
    pub air: bool,
    pub fluid: bool,
    pub state: BlockState,
}

impl<'de> Deserialize<'de> for Row {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RowVisitor;
        impl<'de> Visitor<'de> for RowVisitor {
            type Value = Row;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a registry state row")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Row, A::Error> {
                let length = |i| de::Error::invalid_length(i, &self);
                seq.next_element::<IgnoredAny>()?.ok_or_else(|| length(0))?;
                let air = seq.next_element::<bool>()?.ok_or_else(|| length(1))?;
                let fluid = seq.next_element::<bool>()?.ok_or_else(|| length(2))?;
                seq.next_element::<IgnoredAny>()?.ok_or_else(|| length(3))?;
                let state = seq.next_element::<State>()?.ok_or_else(|| length(4))?;
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Row {
                    air,
                    fluid,
                    state: state.0,
                })
            }
        }
        deserializer.deserialize_seq(RowVisitor)
    }
}

/// A state as BlockState::from_nbt reads it: a name, or `{"id", "properties"}`.
struct State(BlockState);

impl<'de> Deserialize<'de> for State {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Shape {
            Name(String),
            Full {
                id: String,
                #[serde(default)]
                properties: BTreeMap<String, String>,
            },
        }
        Ok(State(match Shape::deserialize(deserializer)? {
            Shape::Name(name) => BlockState {
                name,
                properties: BTreeMap::new(),
            },
            Shape::Full { id, properties } => BlockState {
                name: id,
                properties,
            },
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        DATA_VERSION,
        chunk::BlockState,
        nbt::Tag,
        registry::{BlockRegistry, BlockStateId},
    };
    use serde_json::Value;
    use std::path::Path;

    #[test]
    fn reads_states_and_defaults() {
        let json = br#"{"defaults": {"minecraft:air": 0, "minecraft:oak_slab": 2},
            "states": [
              ["minecraft:air", true, false, "Block{minecraft:air}", "minecraft:air"],
              ["minecraft:oak_slab", false, false, "x", {"id": "minecraft:oak_slab", "properties": {"type": "top"}}, "extra"],
              ["minecraft:oak_slab", false, true, "x", {"id": "minecraft:oak_slab", "properties": {"type": "bottom"}}]
            ], "other": [1, 2]}"#;
        let (registry, flags) = BlockRegistry::from_facts_json_with_flags(&json[..]).unwrap();
        assert_eq!(registry.len(), 3);
        assert_eq!(flags, [(true, false), (false, false), (false, true)]);
        assert_eq!(
            registry.default_state("minecraft:oak_slab"),
            Some(BlockStateId(2))
        );
        let top = BlockState {
            name: "minecraft:oak_slab".into(),
            properties: [("type".into(), "top".into())].into(),
        };
        assert_eq!(registry.id(&top), Some(BlockStateId(1)));
        // A default naming another block, a short row, a missing default.
        for bad in [
            &br#"{"defaults": {"minecraft:air": 1}, "states": [["minecraft:air", true, false, "", "minecraft:air"], ["minecraft:stone", false, false, "", "minecraft:stone"]]}"#[..],
            br#"{"defaults": {"minecraft:air": 0}, "states": [["minecraft:air", true, false, ""]]}"#,
            br#"{"defaults": {}, "states": [["minecraft:air", true, false, "", "minecraft:air"]]}"#,
        ] {
            assert!(BlockRegistry::from_facts_json(bad).is_err());
        }
    }

    /// The streamed registry is the one the JSON and NBT trees built.
    #[test]
    fn matches_the_tree_built_registry() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3/facts.json");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("skipped: no prepared facts.json");
            return;
        };
        fn tag(v: &Value) -> Tag {
            match v {
                Value::String(s) => Tag::String(s.as_str().into()),
                Value::Number(n) => Tag::Int(n.as_i64().unwrap() as i32),
                Value::Object(o) => {
                    Tag::Compound(o.iter().map(|(k, v)| (k.as_str().into(), tag(v))).collect())
                }
                _ => unreachable!(),
            }
        }
        let facts: Value = serde_json::from_slice(&bytes).unwrap();
        let rows = facts["states"].as_array().unwrap();
        let tree = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            (
                "states",
                Tag::List(rows.iter().map(|r| tag(&r[4])).collect()),
            ),
            ("defaults", tag(&facts["defaults"])),
        ]))
        .unwrap();
        let (streamed, flags) = BlockRegistry::from_facts_json_with_flags(&bytes[..]).unwrap();
        assert_eq!(streamed.len(), tree.len());
        for (i, row) in rows.iter().enumerate() {
            let id = BlockStateId(i as u32);
            assert_eq!(streamed.state(id), tree.state(id));
            assert_eq!(
                flags[i],
                (row[1].as_bool().unwrap(), row[2].as_bool().unwrap())
            );
        }
        for name in facts["defaults"].as_object().unwrap().keys() {
            assert_eq!(streamed.default_state(name), tree.default_state(name));
        }
    }
}
