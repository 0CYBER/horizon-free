//! The `standalone_models` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0162

pub trait Hooks {
    /// A player finished editing a sign: `ServerGamePacketListenerImpl.updateSignText`
    /// has taken their update packet and every check has passed (they were
    /// editing that sign, within reach, the sign is not waxed, the chat filter
    /// let the text through), right before the lines go on the sign. `pos` in
    /// `dimension`, the `front` or back side, and the four `lines` as written,
    /// plain text with the formatting codes still in (vanilla strips them
    /// afterwards). Rewrite `lines` (made four lines again) and that is what
    /// the sign gets, without a second look from the chat filter, or set
    /// `cancel` and the sign keeps its old text. With the player's state
    /// swapped in, so `PlayerRef::tell` and the helpers that act on the player
    /// reach them. Once for each update a player sends on a server. Not told:
    /// the host's own editing of a sign on a LAN or single-player world
    /// (the editor writes into their world directly), and the sign changes
    /// that are no edit of the text (dyeing, glow ink, waxing, `/data`,
    /// `/setblock`, mods' `native_with_mod_block_entity`).
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no sign-editing event)",
        cancellable = true
    )]
    fn sign_changed(
        player: u128,
        name: &str,
        dimension: &str,
        pos: [i32; 3],
        front: bool,
        lines: &mut Vec<String>,
        cancel: &mut bool,
    );
    /// Models no blockstate names, registered once on the client before resources load.
    /// `models` collects plain namespaced file ids, baked unturned. `forms` collects
    /// rotated/UV-locked or combined dependencies under a namespaced result id.
    /// Every resource load rebakes both from the active pack files. A missing dependency
    /// drops the entire form and logs its registering mod. Resolve the results through
    /// `RenderAssets::model` after `model_baking_completed`, just like plain ids.
    #[hook(
        side = "client",
        contract = "collect",
        out = "forms",
        answered_by = "mod",
        version = 2,
        context = "registration",
        neoforge = "ModelEvent.RegisterStandalone"
    )]
    fn standalone_models();
}
