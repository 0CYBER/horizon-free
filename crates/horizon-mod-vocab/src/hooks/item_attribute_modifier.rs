//! The `item_attribute_modifier` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `ItemAttributeModifierEvent`: the attribute modifiers `stack` grants
    /// (`ItemStack.getAttributeModifiers`), starting from its own
    /// `minecraft:attribute_modifiers` or its item's. Add, replace or remove
    /// entries in `modifiers`; what is left is what every reader of the stack's
    /// modifiers uses: the equipment of players, mobs, armor stands and the
    /// dragon when it changes, mobs' gear knockback and pickup choices, the
    /// attribute sync and the tooltip. A worn stack is asked again only when it
    /// changes (`ItemStack.matches`), so the answer must be the same for the
    /// same stack. Asked on the server and on a client (its tooltips).
    #[hook(
        side = "both",
        contract = "listen",
        neoforge = "ItemAttributeModifierEvent",
        target = "mod_api::item::item_attributes::native_item_attribute_modifier"
    )]
    fn item_attribute_modifier();
}
