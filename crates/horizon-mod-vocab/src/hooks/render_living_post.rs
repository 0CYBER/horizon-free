//! The `render_living_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After a living entity is drawn (NeoForge's `RenderLivingEvent.Post`): once a frame for each
    /// mob and player in view whose `render_living_pre` wasn't canceled, after its model, its vanilla
    /// layers and the mods' layers. `entity` is its render state, as `render_living_pre` has it;
    /// `draw` is posed at its feet, turned with its body, and what it draws is kept. `parts` is a
    /// humanoid model's parts as vanilla posed them this frame (`HumanoidModel.setupAnim`), taken
    /// from the body's own draw, so a part a pre hook hid reads as not visible; read-only: attach
    /// geometry to an arm or the head without animating it again; None for a model that isn't
    /// humanoid. The order today: before this hook its model, its vanilla layers and the mods'
    /// layers are drawn; after it, in the frame's later passes, its name tag, its leash, its
    /// flames and its shadow (the reference draws the leash and the name tag first, arismoko/horizon#3915). A type
    /// with a mod's registered renderer (`register_entity_renderers`) has its own model's order,
    /// the renderer in the model's place, so it changes with arismoko/horizon#3915 and not before. For a player,
    /// its `render_player_post` follows.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RenderLivingEvent.Post",
        cancellable = false
    )]
    fn render_living_post();
}
