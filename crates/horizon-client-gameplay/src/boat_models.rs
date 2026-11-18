//! The models a boat is drawn with, one part per wood.
use horizon_client::mobs::Part;
use std::collections::HashMap;

pub struct BoatModels {
    pub models: HashMap<String, Part>,
    pub chest_models: HashMap<String, Part>,
    /// `ModelLayers.BOAT_WATER_PATCH`, shared by every boat (not rafts).
    pub water_patch: Option<Part>,
}
impl BoatModels {
    /// Whether a boat of this wood (with or without a chest) can be drawn.
    pub fn has(&self, wood: &str, chest: bool) -> bool {
        if chest {
            self.chest_models.contains_key(wood)
        } else {
            self.models.contains_key(wood)
        }
    }
    pub fn load(document: Option<&horizon_client::mobs::EntityModelDocument>) -> Self {
        let mut models = HashMap::new();
        let mut chest_models = HashMap::new();
        if let Some(document) = document {
            for key in document.keys() {
                if let Some(wood) = key
                    .strip_prefix("minecraft:boat/")
                    .and_then(|k| k.strip_suffix("#main"))
                    && let Some(part) = document.model(key)
                {
                    models.insert(wood.to_owned(), part);
                }
                if let Some(wood) = key
                    .strip_prefix("minecraft:chest_boat/")
                    .and_then(|k| k.strip_suffix("#main"))
                    && let Some(part) = document.model(key)
                {
                    chest_models.insert(wood.to_owned(), part);
                }
            }
        }
        Self {
            models,
            chest_models,
            water_patch: document.and_then(|d| d.model("minecraft:boat#water_patch")),
        }
    }
}
