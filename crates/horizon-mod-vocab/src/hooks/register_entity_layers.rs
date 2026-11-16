//! The `register_entity_layers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The layers a mod adds to living entity renderers (NeoForge's
    /// `EntityRenderersEvent.AddLayers`): `add(target, layer)`, and every living entity of the
    /// type, or every player of the player model, in view draws `EntityRenderLayer::submit` after
    /// its model and the layers added before. `entity_types`, `is_living`, `player_models` and
    /// `has_mod_renderer` tell what renderers this load built. A target that is no living entity
    /// type is refused by name. Asked on the client on every resource load, the first one and each
    /// reload, after the renderers resolve (`register_entity_renderers`): layers don't carry over
    /// from one load to the next, so add them again each time.
    #[notes = r#"## Entity layers

An entity layer (NeoForge's `EntityRenderersEvent.AddLayers` and `RenderLayer`) draws over a living entity
renderer: armour of a mod's own, a glow, something held or worn. `Hooks::register_entity_layers` is asked on
every resource load, after the entity renderers resolve, since a reload builds the renderers again and the
layers don't carry over; the game resolves each target's layers to the slot its draw reads, so a frame
hands an entity to its target's layers only.

```rust,ignore
struct Halo;
impl EntityRenderLayer for Halo {
    fn submit(
        &self,
        _entity: &EntityRenderState,
        _parts: Option<&PosedParts>,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        // In the entity's frame: at its feet, turned with its body. Any atlas sprite will do:
        // a vanilla one, as here, or one of the mod's own assets.
        let face = Some(CubeFace::new("block/gold_block"));
        draw.cube([-4.0, 34.0, -4.0], [4.0, 35.0, 4.0], [face; 6], 0xFFFFFF);
        Ok(())
    }
}

fn register_entity_layers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntityLayersEvent<'_>,
) {
    let halo = Arc::new(Halo);
    for target in [
        LayerTarget::Entity("minecraft:zombie".into()),
        LayerTarget::Player(PlayerModel::Wide),
        LayerTarget::Player(PlayerModel::Slim),
    ] {
        event.layers.add(target, halo.clone()).unwrap();
    }
}
```

The event tells what renderers this load built: `entity_types()` lists every entity type with a renderer,
`is_living(type)` says whether it is a living entity's, `player_models()` lists `Wide` and `Slim`, and
`has_mod_renderer(type)` whether a mod registered the type's renderer. No renderer object crosses the API.

A layer attaches only to a living renderer, a mod's own included: `add` refuses a target that is no entity
type, a type whose renderer isn't a living one (a boat, a display, a projectile), `minecraft:player`,
whose layers go to a player model, and a living type that draws through a route of its own which doesn't
run layers yet (the armour stand, the ender dragon); the `Err` names the target, and the game logs it. An
entity whose `render_living_pre` or `render_player_pre` a mod canceled draws none of its layers, as
NeoForge's canceled `RenderLivingEvent.Pre` skips them. Several mods may add to one target: their layers
draw in the order they were added, after the entity's model and its vanilla layers, each from the entity's
frame (at its feet, turned with its body, as `render_living_pre` draws). `entity.yaw` is the body's turn
and `entity.pitch` the head's; `parts` has a humanoid model's posed parts, to attach to an arm or the
head; they are kept from the model's draw only for a type or player model with a layer (or while a mod
listens to the render hook pair). A layer that returns an `Err`, panics, or (in a wasm mod) has its call
refused or trapped is logged once with the error and asked no more until the next resource load adds it
again; what it drew that frame is dropped. The layers draw on the mobs (vanilla's and the content mods'
living types) and the players; the armour stand and the ender dragon take them once their own draws run
layers, in a follow-up of #860, which lifts their refusal."#]
    #[hook(
        side = "client",
        contract = "listen",
        context = "registration",
        neoforge = "EntityRenderersEvent.AddLayers"
    )]
    fn register_entity_layers();
}
