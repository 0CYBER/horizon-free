//! Startup registration and portable values for mod crafting recipe kinds.
use super::*;
use crate::wire::{Reader, Wire, WireError, Writer};

/// An owned, untrimmed crafting grid. Cells are row-major; empty cells are None.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct CraftingGrid {
    /// Grid width, from one through three.
    pub width: u32,
    /// Grid height, from one through three.
    pub height: u32,
    /// Exactly width times height cells.
    pub cells: Vec<Option<crate::ItemStack>>,
}
impl CraftingGrid {
    /// Validate dimensions and retain an owned snapshot.
    pub fn new(
        width: u32,
        height: u32,
        cells: Vec<Option<crate::ItemStack>>,
    ) -> Result<Self, String> {
        if !(1..=3).contains(&width)
            || !(1..=3).contains(&height)
            || cells.len() != (width * height) as usize
        {
            return Err("invalid crafting grid dimensions".into());
        }
        Ok(Self {
            width,
            height,
            cells,
        })
    }
}

/// A kind's opaque Wire-encoded value. Only its owning handler interprets these bytes.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct CraftingRecipeValue {
    /// The registered value's encoding; the loader admits at most 1 MiB.
    pub bytes: Vec<u8>,
}
impl CraftingRecipeValue {
    /// Retain an encoded value produced by its owning serializer.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

/// Explicit codec for a recipe kind's own value. Omit it to refuse registration.
// no wire: process-local typed codec functions; only their encoded values cross.
#[non_exhaustive]
pub struct CraftingRecipeCodec<T> {
    encode: fn(&T) -> Result<CraftingRecipeValue, String>,
    decode: fn(&CraftingRecipeValue) -> Result<T, String>,
}
impl<T: Wire> CraftingRecipeCodec<T> {
    /// Use the same Wire encoding on native and wasm targets.
    pub fn wire() -> Self {
        Self {
            encode: |value| {
                let mut out = Writer::new();
                out.limit(1024 * 1024);
                value.put(&mut out);
                if let Some(error) = out.error() {
                    return Err(error.to_string());
                }
                Ok(CraftingRecipeValue::new(out.into_bytes()))
            },
            decode: |value| Reader::decode(&value.bytes).map_err(|error| error.to_string()),
        }
    }
}

/// A mod's typed crafting serializer and pure matching callbacks.
/// Register through Hooks::register with its RegistrationContext. Callback failures refuse
/// the file or craft; do not panic. Values must have an explicit Wire codec.
pub trait CraftingRecipeKind: Send + Sync + 'static {
    /// Mod-owned decoded recipe data.
    type Value: Send + Sync + 'static;
    /// Portable codec; the default refuses registration by kind name.
    fn codec(&self) -> Option<CraftingRecipeCodec<Self::Value>> {
        None
    }
    /// Decode the JSON object with its type field removed.
    fn decode(&self, json: &str) -> Result<Self::Value, String>;
    /// Write the decoded value for Horizon client synchronization.
    fn write(&self, value: &Self::Value) -> Result<String, String>;
    /// Whether this value matches the owned grid snapshot.
    fn matches(&self, value: &Self::Value, grid: CraftingGrid) -> bool;
    /// Assemble the result after a successful match. Counts must be 1 through 99;
    /// unknown item keys and larger counts refuse the result.
    fn assemble(&self, value: &Self::Value, grid: CraftingGrid) -> Option<crate::ItemStack>;
    /// One remainder per input cell; None uses the default item remainders.
    /// Asked once per server take, with the grid before consumption. Each count must
    /// be 1 through 99. An invalid key, count or vector length uses defaults.
    fn remaining_items(
        &self,
        value: &Self::Value,
        grid: CraftingGrid,
    ) -> Option<Vec<Option<crate::ItemStack>>> {
        let _ = (value, grid);
        None
    }
}

/// Portable dispatch to the sole owner of a registered kind.
// hidden: type erasure for generated native/wasm registration transport.
#[doc(hidden)]
pub trait CraftingRecipeHandler: Send + Sync {
    /// Decode one recipe object.
    fn decode(&self, json: &str) -> Result<CraftingRecipeValue, String>;
    /// Write one decoded value for sync.
    fn write(&self, value: &CraftingRecipeValue) -> Result<String, String>;
    /// Test this owner's recipe.
    fn matches(&self, value: &CraftingRecipeValue, grid: CraftingGrid) -> bool;
    /// Assemble this owner's recipe.
    fn assemble(&self, value: &CraftingRecipeValue, grid: CraftingGrid)
    -> Option<crate::ItemStack>;
    /// Return custom remainders or the default policy.
    fn remaining_items(
        &self,
        value: &CraftingRecipeValue,
        grid: CraftingGrid,
    ) -> Option<Vec<Option<crate::ItemStack>>>;
}
crate::wire::wire_handlers::handler!(dyn CraftingRecipeHandler, "CraftingRecipeHandler");
struct TypedKind<K: CraftingRecipeKind> {
    kind: K,
    codec: CraftingRecipeCodec<K::Value>,
}
impl<K: CraftingRecipeKind> CraftingRecipeHandler for TypedKind<K> {
    fn decode(&self, json: &str) -> Result<CraftingRecipeValue, String> {
        (self.codec.encode)(&self.kind.decode(json)?)
    }
    fn write(&self, value: &CraftingRecipeValue) -> Result<String, String> {
        self.kind.write(&(self.codec.decode)(value)?)
    }
    fn matches(&self, value: &CraftingRecipeValue, grid: CraftingGrid) -> bool {
        (self.codec.decode)(value).is_ok_and(|value| self.kind.matches(&value, grid))
    }
    fn assemble(
        &self,
        value: &CraftingRecipeValue,
        grid: CraftingGrid,
    ) -> Option<crate::ItemStack> {
        self.kind.assemble(&(self.codec.decode)(value).ok()?, grid)
    }
    fn remaining_items(
        &self,
        value: &CraftingRecipeValue,
        grid: CraftingGrid,
    ) -> Option<Vec<Option<crate::ItemStack>>> {
        self.kind
            .remaining_items(&(self.codec.decode)(value).ok()?, grid)
    }
}
impl dyn Registrar + '_ {
    /// Register a crafting kind once at startup. Duplicate names, foreign namespaces,
    /// and values without an explicit Wire codec refuse startup, naming the kind.
    pub fn register_crafting_recipe_serializer<K: CraftingRecipeKind>(
        &mut self,
        name: &str,
        kind: K,
    ) {
        let handler = typed_handler(kind);
        self.add_recipe_serializer(name, "minecraft:crafting", handler);
    }
}
/// Startup operations for registered crafting kinds.
pub trait RecipeSerializersRegistrar {
    /// Register one owner, or refuse a value without Wire support.
    fn add_recipe_serializer(
        &mut self,
        name: &str,
        recipe_type: &str,
        handler: Option<Arc<dyn CraftingRecipeHandler>>,
    );
    /// Declare a namespaced read-only recipe type.
    fn register_recipe_type(&mut self, name: &str);
}
/// Resolve an installed owner without scanning mods.
pub(crate) fn installed_crafting_recipe(name: &str) -> Option<Arc<dyn CraftingRecipeHandler>> {
    let (owner, recipe_type, handler) = installed()?.keys.recipe_serializers.get(name)?;
    if recipe_type != "minecraft:crafting" {
        return None;
    }
    crate::mod_index_on(*owner).then(|| handler.clone())
}

/// Enumerate enabled kinds at the cold synchronization boundary, including empty kinds.
pub(crate) fn installed_crafting_recipe_kinds() -> Vec<String> {
    let Some(installed) = installed() else {
        return Vec::new();
    };
    let mut kinds: Vec<_> = installed
        .keys
        .recipe_serializers
        .iter()
        .filter(|(_, (owner, recipe_type, _))| {
            crate::mod_index_on(*owner) && recipe_type == "minecraft:crafting"
        })
        .map(|(kind, _)| kind.to_string())
        .collect();
    kinds.sort();
    kinds
}

/// Resolve a serializer and its separately registered recipe type.
pub(crate) fn installed_recipe_serializer(
    name: &str,
) -> Option<(String, Arc<dyn CraftingRecipeHandler>)> {
    let (owner, recipe_type, handler) = installed()?.keys.recipe_serializers.get(name)?;
    (crate::mod_index_on(*owner)
        && (recipe_type == "minecraft:crafting"
            || installed()?
                .keys
                .recipe_types
                .contains_key(recipe_type.as_str())))
    .then(|| (recipe_type.clone(), handler.clone()))
}

pub(super) fn typed_handler<K: CraftingRecipeKind>(
    kind: K,
) -> Option<Arc<dyn CraftingRecipeHandler>> {
    kind.codec()
        .map(|codec| Arc::new(TypedKind { kind, codec }) as Arc<dyn CraftingRecipeHandler>)
}

pub(crate) fn installed_recipe_serializers() -> Vec<(String, String)> {
    installed().map_or_else(Vec::new, |installed| {
        installed
            .keys
            .recipe_serializers
            .iter()
            .filter(|(_, (owner, _, _))| crate::mod_index_on(*owner))
            .map(|(id, (_, kind, _))| (id.to_string(), kind.clone()))
            .collect()
    })
}
