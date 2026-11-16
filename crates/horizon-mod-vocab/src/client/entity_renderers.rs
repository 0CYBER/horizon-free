//! Code-defined entity renderers, as a client mod registers them in `register_entity_renderers`
//! (NeoForge's `EntityRenderersEvent.RegisterRenderers` and `EntityRenderer`).
//!
//! A renderer draws every entity of one entity type, named by its id (`mymod:bolt`). The game
//! keeps the registrations from start and resolves each type's renderer on every resource load, so
//! a frame hands an entity to the renderer of its type's slot and to no other mod.
use crate::BlockEntityDrawContext;
use crate::wire::{Reader, Wire, WireError, Writer};
use std::{collections::BTreeMap, sync::Arc};

/// One entity's frame, as `EntityRenderer.extractRenderState` fills an `EntityRenderState`:
/// what a renderer reads to place and turn its model. Angles are in degrees, interpolated to the
/// frame's partial tick. Plain numbers, so a frame allocates nothing, and one value for native and
/// wasm mods alike.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub struct EntityRenderState {
    /// The entity's network id, as mod messages name it.
    pub id: u32,
    /// The frame's partial tick, 0 to 1.
    pub partial: f32,
    /// `ageInTicks`: ticks lived plus the partial tick, for motion of its own.
    pub age: f32,
    /// `Entity.getYRot` at the frame: the way it faces (0 is south, 90 west).
    pub yaw: f32,
    /// `Entity.getXRot` at the frame: its pitch, positive looking down.
    pub pitch: f32,
    /// A living entity's own frame (`LivingEntityRenderState`); none for any other entity.
    pub living: Option<LivingRenderState>,
}

/// What a living entity's frame adds to [`EntityRenderState`], as
/// `LivingEntityRenderer.extractRenderState` fills a `LivingEntityRenderState`: what a model
/// reads to pose and animate a body. Angles are in degrees, interpolated to the frame's partial
/// tick. Plain values, so a frame allocates nothing, and one value for native and wasm mods alike.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub struct LivingRenderState {
    /// `bodyRot`: the way the body faces (0 is south, 90 west).
    pub body_rot: f32,
    /// `yRot`: the head's turn from the body, -180 to 180 (its sign flipped when
    /// `is_upside_down`).
    pub y_rot: f32,
    /// `xRot`: the head's pitch, positive looking down (its sign flipped when `is_upside_down`).
    pub x_rot: f32,
    /// `deathTime`: ticks since it died plus the partial tick, 0 while alive (vanilla tips the
    /// body over the first 20).
    pub death_time: f32,
    /// `walkAnimationPos`: how far the legs have swung, for a walk cycle (a vanilla quadruped's
    /// legs swing `cos(pos * 0.6662) * 1.4 * speed` radians); 0 riding or dead.
    pub walk_animation_pos: f32,
    /// `walkAnimationSpeed`: how fast it walks, 0 standing to 1; 0 riding or dead.
    pub walk_animation_speed: f32,
    /// `scale`: the entity's `minecraft:scale` (`LivingEntity.getScale`), by which vanilla scales
    /// the whole model.
    pub scale: f32,
    /// `ageScale`: 0.5 for a baby, else 1 (`LivingEntity.getAgeScale`).
    pub age_scale: f32,
    /// `isUpsideDown`: named Dinnerbone or Grumm, drawn head down.
    pub is_upside_down: bool,
    /// `isBaby`.
    pub is_baby: bool,
    /// `isInWater`.
    pub is_in_water: bool,
    /// `hasRedOverlay`: hurt or dying, drawn tinted red.
    pub has_red_overlay: bool,
    /// `pose`: its stance (`Entity.getPose`).
    pub pose: crate::Pose,
    /// `bedOrientation`: the way the bed it sleeps in faces, while it sleeps in one.
    pub bed_orientation: Option<crate::Direction>,
}

/// A mod's entity renderer: NeoForge's `EntityRenderer`, held by the game for every entity of the
/// type it is registered for. Entities of a type share one renderer, so what differs between them
/// comes from [`EntityRenderState`].
///
/// A frame has no `Host`: drawing reads nothing of the world but what the state and `draw` give.
/// A renderer that needs more reads it in a hook and keeps what it read.
pub trait EntityRenderer: Send + Sync + 'static {
    /// `EntityRenderer.submit`: draw `entity` with `draw`'s helpers (`quad`, `cube`, `block`,
    /// `item`, `model_part`) under its pose stack, which starts at the entity's position, not
    /// turned: turn it by `entity.yaw` and `entity.pitch` as the model needs. `draw` also has
    /// the partial tick, the game time and the entity's light. Asked once a frame for each entity
    /// of the type in view; what it draws replaces the type's own drawing. A frame is drawn
    /// afresh: `draw.keep()` does nothing here. A native implementation returns `Ok(())` in the
    /// normal case, and that keeps what it drew. Diverges: Horizon's `submit` returns a `Result`
    /// so a refused or trapped call is visible. An `Err`, a panic, and a wasm call that is
    /// refused or traps all mean the same: what it drew this frame is dropped, the error is
    /// logged once, and the renderer is asked no more until the next resource load resolves it
    /// again; until then its type draws as it would without one.
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String>;
}

crate::wire::wire_handlers::handler!(dyn EntityRenderer, "EntityRenderer");

/// The entity renderer registry `register_entity_renderers` hands out: NeoForge's
/// `EntityRenderersEvent.RegisterRenderers`.
pub trait EntityRenderers {
    /// `registerEntityRenderer(type, provider)`: `renderer` draws every entity of the type
    /// `entity_type` (`mymod:bolt`). A later registration for a type replaces the earlier one,
    /// whichever mod made it. A type the game does not know is logged with its id when resources
    /// load, and nothing is drawn by it.
    fn register(&mut self, entity_type: String, renderer: Arc<dyn EntityRenderer>);
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`EntityRenderers`]: the registrations by entity type.
#[derive(Default)]
// exhaustive: its field is private; the game builds it
pub struct EntityRendererRegistry {
    renderers: BTreeMap<String, Arc<dyn EntityRenderer>>,
}

impl EntityRenderers for EntityRendererRegistry {
    fn register(&mut self, entity_type: String, renderer: Arc<dyn EntityRenderer>) {
        self.renderers.insert(entity_type, renderer);
    }
}

impl EntityRendererRegistry {
    /// Each type's renderer, the last registration for a type winning.
    pub fn into_renderers(self) -> BTreeMap<String, Arc<dyn EntityRenderer>> {
        self.renderers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Marked(u32);
    impl EntityRenderer for Marked {
        fn submit(
            &self,
            _entity: &EntityRenderState,
            draw: &mut dyn BlockEntityDrawContext,
        ) -> Result<(), String> {
            draw.set_light(self.0);
            Ok(())
        }
    }

    #[derive(Default)]
    struct Light(u32);
    impl BlockEntityDrawContext for Light {
        fn set_light(&mut self, light: u32) {
            self.0 = light;
        }
    }

    #[test]
    fn a_later_registration_for_a_type_replaces_the_earlier_one() {
        let mut registry = EntityRendererRegistry::default();
        registry.register("mymod:bolt".into(), Arc::new(Marked(1)));
        registry.register("mymod:dart".into(), Arc::new(Marked(2)));
        registry.register("mymod:bolt".into(), Arc::new(Marked(3)));
        let renderers = registry.into_renderers();
        assert_eq!(renderers.len(), 2);
        let state = EntityRenderState {
            id: 7,
            partial: 0.5,
            age: 3.5,
            yaw: 0.0,
            pitch: 0.0,
            living: None,
        };
        let (mut bolt, mut dart) = (Light::default(), Light::default());
        renderers["mymod:bolt"].submit(&state, &mut bolt).unwrap();
        renderers["mymod:dart"].submit(&state, &mut dart).unwrap();
        assert_eq!((bolt.0, dart.0), (3, 2));
    }
}
