//! The `item_model_update` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A node of a type a mod registered (`register_item_models`, vanilla's `ItemModel.update`)
    /// is resolved for a stack, in hand, in a screen or on the ground: push the names of the
    /// node's `models` to draw, in order, only for a type your mod owns. `config` borrows the
    /// node's JSON object, converted once at load. The owned stack snapshot is the one being
    /// resolved, including nested bundle items. `bundle_selected` is the runtime bundle selection
    /// index; -1 means none. As `ItemModel.update` reads them: `display` is where the item is
    /// drawn (a screen, a hand, the ground), `owner` the living entity holding it (`None` in a
    /// screen with no holder), `level` the level it is drawn in (`None` outside a world), and
    /// `seed` the draw's random seed.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla ItemModel.update of a type RegisterItemModelsEvent registers; NeoForge adds no event for it)"
    )]
    fn item_model_update(
        kind: &str,
        config: &Tag,
        stack: &ItemStack,
        bundle_selected: i32,
        display: ItemDisplayContext,
        owner: Option<&ItemModelOwner>,
        level: Option<&ItemModelLevel>,
        seed: i32,
        models: &mut Vec<String>,
    );
}
