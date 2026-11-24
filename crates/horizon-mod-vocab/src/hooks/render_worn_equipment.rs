//! The `render_worn_equipment` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// What a living entity wears and holds, as it is drawn this frame: asked once a frame for
    /// each mob and player in view, this window's own included, right after `render_living_pre`
    /// when neither it nor a player's `render_player_pre` canceled. `entity` is its render state,
    /// as `render_living_pre` has it. `equipment` starts with everything shown and untinted:
    /// `hide(slot, part)` leaves a slot's base (the piece, with its glint and trim), glint or
    /// trim out this frame, and `tint(slot, rgb)` multiplies the colour the slot draws in (the
    /// piece, its dyed and emissive layers and its trim, not its glint). The slots are the four
    /// armour slots (the head's covers the item `CustomHeadLayer` draws there too), the body
    /// (a wolf's, horse's or nautilus's armour, a llama's carpet, a happy ghast's harness), the
    /// elytra and both hands. Only what Horizon draws for a slot changes: a mob's armour has no
    /// trim drawn, a body armour no glint. The entity's model, its other layers, the cape and the
    /// mods' layers stay as they were, and so do the screens' previews of the player. It is a
    /// hook of its own rather than a field of `RenderLivingPreEvent` so that wasm mods built
    /// before it keep loading.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn render_worn_equipment();
}
