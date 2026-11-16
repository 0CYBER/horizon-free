//! Authoritative item requirements supplied by the custom recipe type.
use crate::Ingredient;

/// Vanilla's `Recipe#placementInfo`: ingredient cells, distinct from display
/// pictures. Supplied by the same recipe-type-owned provider as its displays.
/// NeoForge public documentation names no separate registration event.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
// exhaustive: recipe providers construct the complete placement declaration
pub struct PlacementInfo {
    /// Ordered recipe cells. `None` is a shaped hole, never a wildcard ingredient.
    /// An empty list is `NOT_PLACEABLE`. The load boundary validates every
    /// ingredient against the actual item registry before making it available.
    pub cells: Vec<Option<Ingredient>>,
    /// Shaped width and height, or `None` for consecutive inputs. For named
    /// station roles, cells follow input order: furnace/stonecutter input;
    /// smithing template, base, addition. Fuel is never an input here.
    pub shape: Option<[u16; 2]>,
}

impl PlacementInfo {
    /// The recipe cannot be represented as item placement requirements. Such
    /// recipes may still display, but placement requests refuse without mutation.
    pub const NOT_PLACEABLE: Self = Self {
        cells: Vec::new(),
        shape: None,
    };
}
