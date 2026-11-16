//! The `animal_tame` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The player `tamer` (UUID) and `tamer_entity_id` is about to tame the
    /// mob `entity` of `kind` (`AnimalTameEvent`): its taming roll succeeded
    /// (a wolf's bone, a cat's fish, a parrot's seeds or a nautilus's food;
    /// a horse, donkey, mule or llama the player rides). Set `cancel` for the
    /// attempt to fail instead, as a failed roll does: smoke for a pet, and
    /// a horse bucks its rider off. Before `entity_tame`. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "AnimalTameEvent",
        paper = "EntityTameEvent",
        cancellable = true
    )]
    fn animal_tame();
}
