//! Java 26.3 compact/full block-state JSON codecs over a pinned registry.
use super::{BlockRegistry, BlockStateId};
use crate::compression::invalid;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// Property kinds accompany the imported state registry. Integer properties
/// parse Java decimal strings; enum and boolean properties use exact names.
pub struct BlockStateJsonCodec<'a> {
    registry: &'a BlockRegistry,
    integers: BTreeSet<(String, String)>,
    counts: BTreeMap<String, usize>,
}
impl<'a> BlockStateJsonCodec<'a> {
    pub fn new(
        registry: &'a BlockRegistry,
        integers: BTreeSet<(String, String)>,
    ) -> io::Result<Self> {
        for (block, property) in &integers {
            let state = registry
                .default_state(block)
                .and_then(|id| registry.state(id));
            if !state.is_some_and(|state| state.properties.contains_key(property)) {
                return Err(invalid("integer property is absent from block registry"));
            }
        }
        let mut counts = BTreeMap::new();
        for state in &registry.states {
            *counts.entry(state.name.clone()).or_insert(0) += 1;
        }
        Ok(Self {
            registry,
            integers,
            counts,
        })
    }
    /// `full` selects FULL_CODEC, which requires the object representation.
    /// Invalid/missing properties keep their defaults, as vanilla's codec does.
    pub fn decode(&self, json: &str, full: bool) -> io::Result<BlockStateId> {
        let value: Value = serde_json::from_str(json).map_err(|e| invalid(e.to_string()))?;
        let name = if !full && value.is_string() {
            value.as_str()
        } else {
            value.get("id").and_then(Value::as_str)
        }
        .ok_or_else(|| invalid("missing block identifier"))?;
        let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
        let name = format!(
            "{}:{path}",
            if namespace.is_empty() {
                "minecraft"
            } else {
                namespace
            }
        );
        let default = self
            .registry
            .default_state(&name)
            .ok_or_else(|| invalid("unknown block identifier"))?;
        let mut state = self.registry.state(default).unwrap().clone();
        if let Some(properties) = value.get("properties").and_then(Value::as_object) {
            for (key, raw) in properties {
                if !state.properties.contains_key(key) {
                    continue;
                }
                let Some(value) = raw.as_str() else {
                    continue;
                };
                let candidate = if self.integers.contains(&(name.clone(), key.clone())) {
                    let Some(value) = java_integer(value) else {
                        continue;
                    };
                    value.to_string()
                } else {
                    value.to_owned()
                };
                let previous = state.properties.insert(key.clone(), candidate).unwrap();
                if self.registry.id(&state).is_none() {
                    state.properties.insert(key.clone(), previous);
                }
            }
        }
        self.registry
            .id(&state)
            .ok_or_else(|| invalid("block state is absent from registry"))
    }
    pub fn encode(&self, id: BlockStateId, full: bool) -> io::Result<String> {
        let state = self
            .registry
            .state(id)
            .ok_or_else(|| invalid("unknown block state ID"))?;
        let value = if !full && self.registry.default_state(&state.name) == Some(id) {
            Value::String(state.name.clone())
        } else if self.counts[&state.name] == 1 {
            serde_json::json!({"id":state.name})
        } else {
            serde_json::json!({"id":state.name,"properties":state.properties})
        };
        serde_json::to_string(&value).map_err(|e| invalid(e.to_string()))
    }
}

fn java_integer(text: &str) -> Option<i32> {
    let (negative, text) = if let Some(text) = text.strip_prefix('-') {
        (true, text)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    if text.is_empty() {
        return None;
    }
    // BMP decimal digit runs accepted by Character.digit(char, 10) in JDK 25.
    const ZEROES: [u32; 37] = [
        0x30, 0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66,
        0xde6, 0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90,
        0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0,
        0xff10,
    ];
    let mut value = 0i64;
    for c in text.chars() {
        let c = c as u32;
        let zero = ZEROES
            .iter()
            .copied()
            .find(|zero| c >= *zero && c < *zero + 10)?;
        value = value * 10 + i64::from(c - zero);
        if value > i64::from(i32::MAX) + i64::from(negative) {
            return None;
        }
    }
    Some(if negative { -value } else { value } as i32)
}
