//! Read-only serializers share registration and portable values with crafting serializers.
use super::*;
/// A serializer for a separately registered, mod-read recipe type.
/// The engine publishes its source JSON through recipes_of_type and never matches it.
pub trait RecipeSerializer: Send + Sync + 'static {
    /// Mod-owned portable value.
    type Value: Send + Sync + 'static;
    /// Missing Wire support refuses registration by serializer name.
    fn codec(&self) -> Option<crate::CraftingRecipeCodec<Self::Value>> {
        None
    }
    /// Decode the JSON object without type; a refusal skips this known recipe alone.
    fn decode(&self, json: &str) -> Result<Self::Value, String>;
    /// Write the decoded value for synchronization.
    fn write(&self, value: &Self::Value) -> Result<String, String>;
}
struct ReadOnly<S>(S);
impl<S: RecipeSerializer> crate::CraftingRecipeKind for ReadOnly<S> {
    type Value = S::Value;
    fn codec(&self) -> Option<crate::CraftingRecipeCodec<Self::Value>> {
        self.0.codec()
    }
    fn decode(&self, json: &str) -> Result<Self::Value, String> {
        self.0.decode(json)
    }
    fn write(&self, value: &Self::Value) -> Result<String, String> {
        self.0.write(value)
    }
    fn matches(&self, _: &Self::Value, _: crate::CraftingGrid) -> bool {
        false
    }
    fn assemble(&self, _: &Self::Value, _: crate::CraftingGrid) -> Option<crate::ItemStack> {
        None
    }
}
struct Raw;
impl RecipeSerializer for Raw {
    type Value = String;
    fn codec(&self) -> Option<crate::CraftingRecipeCodec<String>> {
        Some(crate::CraftingRecipeCodec::wire())
    }
    fn decode(&self, json: &str) -> Result<String, String> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        value
            .is_object()
            .then(|| json.to_owned())
            .ok_or_else(|| "recipe must be an object".into())
    }
    fn write(&self, value: &String) -> Result<String, String> {
        Ok(value.clone())
    }
}
impl dyn Registrar + '_ {
    /// Register a serializer for an independently declared read-only recipe type.
    pub fn register_read_only_recipe_serializer<S: RecipeSerializer>(
        &mut self,
        name: &str,
        recipe_type: &str,
        serializer: S,
    ) {
        let kind = ReadOnly(serializer);
        let handler = crate::parts::crafting_recipes::typed_handler(kind);
        self.add_recipe_serializer(name, recipe_type, handler);
    }
    /// Register an object-preserving JSON serializer for a mod-read machine recipe type.
    /// Declare recipe_type with register_recipe_type first (or elsewhere at startup).
    pub fn register_raw_recipe_serializer(&mut self, name: &str, recipe_type: &str) {
        self.register_read_only_recipe_serializer(name, recipe_type, Raw);
    }
}
