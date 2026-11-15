//! Model loaders native mods register (NeoForge's `ModelEvent.RegisterLoaders`): a model whose
//! `loader` names one keeps its JSON object, and the geometry its mod answers is baked with the
//! model's elements. The answer comes through [`LoaderGeometry`], which the client supplies, so
//! this crate never reaches the mod API.
use crate::model::Direction;
use serde_json::Value;
use std::{collections::BTreeSet, sync::RwLock};

#[cfg(test)]
#[path = "model_loaders_tests.rs"]
mod tests;

static TYPES: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());

/// Keeps the loader ids native mods registered, before resources load: an id that is not
/// `<namespace>:<path>` in a namespace other than vanilla's, or one registered already, is logged
/// and left out. Returns the ids kept, in registration order.
pub fn register(ids: Vec<String>) -> Vec<String> {
    crate::item_model::keep_mod_ids(&TYPES, "model loader", ids)
}

/// Whether a native mod registered `id` as a model loader.
pub fn is_registered(id: &str) -> bool {
    TYPES.read().unwrap_or_else(|e| e.into_inner()).contains(id)
}

/// One quad a registered loader adds, checked by the client before it gets here.
#[derive(Clone, Debug, PartialEq)]
pub struct LoaderQuad {
    /// Corners in block units, as an obj model's are.
    pub positions: [[f32; 3]; 4],
    /// Each corner's texture coordinates, 0 to 1 across the sprite.
    pub uvs: [[f32; 2]; 4],
    /// A key of the model's `textures`, without `#`.
    pub texture: String,
    /// The face whose solid neighbour hides it; `None` never culls.
    pub cull: Option<Direction>,
    pub tint_index: i32,
    pub shade: bool,
}

/// The quads a registered loader answers for a model, asked once per model each time a library
/// resolves it. `model` names the model for the log.
pub trait LoaderGeometry: Send + Sync {
    fn quads(&self, loader: &str, model: &str, config: &Value) -> Vec<LoaderQuad>;
}
