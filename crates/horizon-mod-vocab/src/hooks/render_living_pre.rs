//! The `render_living_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a living entity is drawn (NeoForge's `RenderLivingEvent.Pre`): once a frame for each
    /// mob and player in view (one the frame culls is not asked), before its pose, model and
    /// layers. `entity` is its render state (its id, kind, custom name, the frame's partial tick,
    /// its body and head turns and walk cycle, `LivingEntityRenderState`). `draw` is posed at its
    /// feet, turned with its body: +z the way it faces, +x its left, +y up, in blocks, with the
    /// helpers block entities draw with (`cube`, `quad`, `block`, `item`, `model_part`); what it
    /// draws is kept. Set `canceled` to skip all of the entity's drawing this frame: its model,
    /// armour, held items and layers, the mods' layers, its name tag, shadow, flames, leash and
    /// glowing outline, and `render_living_post`, and the renderer a mod registered for its type
    /// (`register_entity_renderers`), which draws in the model's place; a mod drawing a model of
    /// its own draws it here and cancels. A player's `render_player_pre` comes first, and a cancel here still asks its
    /// `render_player_post`. Left uncanceled, `hidden.hide_named(name)` leaves only the part of
    /// that name out of the vanilla model this frame (a villager's `nose`), drawn on the CPU or the
    /// GPU: a hidden part takes its children with it and leaves the layers drawn on the model itself
    /// (`getParentModel()`: a villager's type, profession and level, a spider's eyes), while its
    /// armour, held items and other layers keep every part. An entity is drawn afresh each frame:
    /// `draw.keep()` does nothing here.
    #[hook(
        side = "client",
        version = 2,
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderLivingEvent.Pre",
        cancellable = true
    )]
    fn render_living_pre();
}
