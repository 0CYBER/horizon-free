//! Fork-shaped conversion of the upstream declaration.

pub trait Hooks {
    /// a right-click on an entity with the point clicked: `entity_interact_specific` hears the entity, its type, the held stack and the point clicked, measured from the entity's position, before `native_interact_entity` (NeoForge's `PlayerInteractEvent.EntityInteractSpecific` counterpart); a claim swings the hand and ends the click. Server-side. Bundles need "hooks": @API:entity_interact_specific@.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "claimed",
        neoforge = "PlayerInteractEvent.EntityInteractSpecific"
    )]
    fn entity_interact_specific();
}
