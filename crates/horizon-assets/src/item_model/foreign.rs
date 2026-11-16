//! Properties of other namespaces (a mod's, NeoForge's) that no mod registered decode to the
//! closest vanilla behaviour. A node type no mod registered fails to decode instead
//! (`ItemModelNode::decode`).
use super::{ItemModelNode, Result, ident};
use crate::{json::error as err, math::Mat4};
use serde_json::{Map, Value};

pub fn is_foreign(id: &str) -> bool {
    !id.starts_with("minecraft:")
}

/// A composite carries the transformation a collapsed node had.
pub fn wrap(node: ItemModelNode, transformation: Option<Mat4>) -> ItemModelNode {
    match transformation {
        None => node,
        Some(_) => ItemModelNode::Composite {
            models: vec![node],
            transformation,
        },
    }
}

pub fn property_is_foreign(m: &Map<String, Value>) -> Result<bool> {
    let property = m
        .get("property")
        .ok_or_else(|| err("missing property"))
        .and_then(ident)?;
    Ok(is_foreign(&property))
}
