//! The `register_entity_renderers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The code-defined renderers of entity types (NeoForge's
    /// `EntityRenderersEvent.RegisterRenderers`, `registerEntityRenderer`): `register(type,
    /// renderer)`, and every entity of the type `type` (`mymod:bolt`) in view is drawn by
    /// `EntityRenderer::submit` in place of the type's own drawing. A later registration for a
    /// type replaces the earlier one; a type the game does not know is logged with its id and
    /// draws nothing new. Asked once at start on the client, before resources load; the game
    /// resolves each type's renderer on every resource load. This build draws the registered
    /// renderers of content mods' thrown types and of the living types drawn as mobs (in place of
    /// the model and its vanilla layers); the other entity families keep their own drawing.
    #[notes = r#"## Entity renderers

An entity renderer (NeoForge's `EntityRenderersEvent.RegisterRenderers` and `EntityRenderer`) gives an
entity type code that draws it. `Hooks::register_entity_renderers` registers one renderer per entity type
once at start, before resources load, and the game resolves each type's renderer on every resource load
(the first one and every reload after it), so a frame hands an entity to the renderer of its type: no
lookup by name, no scan over the mods.

```rust,ignore
struct Bolt;
impl EntityRenderer for Bolt {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        // The pose starts at the entity, not turned: face the way it flies.
        draw.rotate_about([0.0, 1.0, 0.0], -entity.yaw, [0.0; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], entity.pitch, [0.0; 3]);
        let face = Some(CubeFace::new("example:entity/bolt"));
        draw.cube([-1.0, -1.0, -4.0], [1.0, 1.0, 4.0], [face; 6], 0xFFFFFF);
        Ok(())
    }
}

fn register_entity_renderers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntityRenderersEvent<'_>,
) {
    event.renderers.register("example:bolt".into(), Arc::new(Bolt));
}
```

The renderer draws with the helpers a block entity or mob submit hook draws with (`quad`, `cube`, `block`,
`item`, `model_part`), under a pose stack that starts at the entity's interpolated position, in blocks,
not turned; `EntityRenderState` has the frame's partial tick, the entity's age and its interpolated yaw
and pitch, and `draw.light()` the entity's light. A living entity's state also has `living`, its
`LivingRenderState`: body and head turns, walk cycle, death time, scale, pose, bed and flags, by the
reference's names. What it draws replaces the type's own drawing (a thrown type's flying item; a living
type's model and its vanilla layers, held items and worn blocks included); the glowing outline still
draws around it. Entities of a type share the one
renderer, which has no `Host` and no level: a renderer that needs the world reads it in a hook. The same
for native and wasm mods: a wasm renderer is one call a frame for each entity, the state copied in and
the drawing through the shared draw boundary.

A later registration for a type replaces the earlier one, whichever mod made it, as in NeoForge. When
resources load, a registration for a type the game does not know is logged with the type's id and left
out; one for a known type whose family does not draw through registered renderers yet is kept silently,
and the type keeps its own drawing until its family does. Today two routes draw through registered
renderers: the content mods' thrown types (a `data/<id>/horizon/entity_type/<name>.json` with the
`minecraft:snowball` template), and the living types drawn as mobs, vanilla's and the content mods'
(not the player, whose model goes by its skin, and not the armour stand or the ender dragon, which draw
through routes of their own). Around a living type's renderer everything else stays: `render_living_pre`
(whose cancel skips it), the mods' layers (with no posed parts) and `render_living_post`, in the order
the type's own model has, so it changes with #3915 and not before. Boats, displays and the other
projectiles come in follow-ups of #860, and a mod's general entity types (#792) will register the same
way. A renderer that returns an `Err`,
panics, or (in a wasm mod) has its call refused or trapped is logged once with the error and asked no
more until the next resource load resolves it again; what it drew that frame is dropped, and until then
its type draws as it would without one."#]
    #[hook(
        side = "client",
        contract = "listen",
        context = "registration",
        neoforge = "EntityRenderersEvent.RegisterRenderers"
    )]
    fn register_entity_renderers();
}
