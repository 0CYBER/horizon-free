//! Layers on living entity renderers, as a client mod adds them in `register_entity_layers`
//! (NeoForge's `EntityRenderersEvent.AddLayers` and `RenderLayer`).
//!
//! A layer draws over a living entity of one type, or over every player of one player model, after
//! its model and the layers before it. The game asks for the layers on every resource load, as the
//! renderers are built again, and resolves each target's layers to the slot its draw reads, so a
//! frame hands an entity to its own type's layers and to no other mod.
use crate::wire::{Reader, Wire, WireError, Writer};
use crate::{BlockEntityDrawContext, EntityRenderState, PosedParts};
use std::sync::Arc;

/// A player model, as a player's skin chooses it: each has a renderer of its own, and a layer added
/// to one draws on every player using it (NeoForge's `PlayerModelType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum PlayerModel {
    /// Steve's arms, four pixels wide.
    Wide,
    /// Alex's arms, three pixels wide.
    Slim,
}

/// What a layer is added to.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum LayerTarget {
    /// Every entity of a living entity type (`minecraft:zombie`, `mymod:golem`).
    Entity(String),
    /// Every player of a player model.
    Player(PlayerModel),
}

/// A mod's layer: NeoForge's `RenderLayer`, held by the game for every entity of its target.
/// Entities of a target share one layer, so what differs between them comes from
/// [`EntityRenderState`].
///
/// A frame has no `Host`: drawing reads nothing of the world but what the state, the parts and
/// `draw` give. A layer that needs more reads it in a hook and keeps what it read.
pub trait EntityRenderLayer: Send + Sync + 'static {
    /// `RenderLayer.submit`: draw over `entity` with `draw`'s helpers (`quad`, `cube`, `block`,
    /// `item`, `model_part`), after its model and the layers before this one. The pose stack starts
    /// in the entity's frame: at its feet, turned with its body, +z the way it faces, +x its left,
    /// +y up, in blocks (`entity.yaw` is the body's turn, `entity.pitch` the head's). `parts` are a
    /// humanoid model's parts as vanilla posed them this frame, to attach to an arm or the head;
    /// None for a model that isn't humanoid. `draw` also has the partial tick, the game time and
    /// the entity's light. Asked once a frame for each entity of the target in view, but not for
    /// one whose `render_living_pre` or `render_player_pre` a mod canceled: as with NeoForge's
    /// canceled `RenderLivingEvent.Pre`, none of its layers draw then. A frame is
    /// drawn afresh: `draw.keep()` does nothing here. `Ok(())` keeps what it drew. An `Err`, a
    /// panic, and a wasm call that is refused or traps all mean the same: what it drew this frame
    /// is dropped, the error is logged once, and the layer is asked no more until the next
    /// resource load adds it again.
    fn submit(
        &self,
        entity: &EntityRenderState,
        parts: Option<&PosedParts>,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String>;
}

crate::wire::wire_handlers::handler!(dyn EntityRenderLayer, "EntityRenderLayer");

/// The layer registry `register_entity_layers` hands out: NeoForge's
/// `EntityRenderersEvent.AddLayers`, with what it tells of the renderers this load built.
pub trait EntityLayers {
    /// `getEntityTypes`: every entity type that has a renderer this load, vanilla's and the mods'
    /// (`minecraft:oak_boat`, `mymod:bolt`), sorted. A player has none of its own: its renderers
    /// are the player models'.
    fn entity_types(&self) -> Vec<String>;
    /// Whether `entity_type`'s renderer is a living entity's, the only kind that takes layers.
    /// False for a type with no renderer.
    fn is_living(&self, entity_type: &str) -> bool;
    /// `getSkins`: the player models, each with a renderer of its own.
    fn player_models(&self) -> Vec<PlayerModel>;
    /// Whether a mod registered `entity_type`'s renderer (`register_entity_renderers`), rather than
    /// it being the game's own.
    fn has_mod_renderer(&self, entity_type: &str) -> bool;
    /// `addLayer`: `layer` draws over every entity of `target`, after the layers added to it
    /// before, whichever mod added them. A target that can't take layers is refused: not an entity
    /// type, a type whose renderer isn't a living one, `minecraft:player` (whose layers go to a
    /// player model), and a living type that draws through a route of its own that doesn't run
    /// layers yet (the armour stand, the ender dragon). The `Err` names it, and the game logs it
    /// too.
    fn add(&mut self, target: LayerTarget, layer: Arc<dyn EntityRenderLayer>)
    -> Result<(), String>;
}
