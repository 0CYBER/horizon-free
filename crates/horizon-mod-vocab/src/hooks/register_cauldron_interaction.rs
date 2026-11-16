pub trait Hooks {
    /// Register cauldron item interactions, by exact item or by item tag, on both sides before
    /// snapshot publication. Vanilla dye tags take precedence, then exact items, then item tags
    /// in registration order; duplicate entries refuse preparation.
    #[notes = "## Cauldron interactions

`register_cauldron_interaction` registers item and item-tag callbacks on both sides. See [Registering cauldron interactions](../how-to/world/cauldron-interactions.md) for the read-only decision, transactional fire, duplicate refusal and server-corrected prediction contract."]
    #[hook(
        side = "both",
        contract = "listen",
        context = "registration",
        neoforge = "RegisterCauldronInteractionEvent"
    )]
    fn register_cauldron_interaction();
}
