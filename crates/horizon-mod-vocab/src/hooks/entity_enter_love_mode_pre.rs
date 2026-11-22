//! The `entity_enter_love_mode_pre` hook for the mod API generator.

pub trait Hooks {
    /// An eligible adult `animal` is about to enter love mode in `level`
    /// after feeding. `feeder` is the player when known. Cancel to skip
    /// the love timer, breeding credit, entry hearts and
    /// [`Hooks::entity_enter_love_mode`]. Feeding still follows its normal
    /// path, including consumption and other effects. Runs for ordinary
    /// animals, equines, llamas, pandas and nautili. If a listener moves
    /// the animal to another level or removes it, its feeding ends there.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_enter_love_mode_pre(
        animal: EntityRef,
        level: LevelRef,
        feeder: Option<PlayerRef>,
        canceled: &mut bool,
    );
}
