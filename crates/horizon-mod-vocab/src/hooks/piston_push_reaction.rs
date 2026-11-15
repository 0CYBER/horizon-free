//! State-only piston reaction override.
pub trait Hooks {
    /// Resolve a state's piston reaction at load and reload on both sides. None keeps the
    /// properties' reaction. Answers must depend only on state. Unbreakable blocks still
    /// refuse movement. The reaction is checked first; push-pull, ignore-entity and push only
    /// then reach the shared block-entity rule. An otherwise-pushable block with a block entity
    /// moves only when `piston_carries_block_entity` answers true, using the carry machinery
    /// to move its data and attachments without drops. No answer or false refuses the move.
    /// Popped remains destruction with ordinary drops, and never asks the carries hook.
    /// The carries hook is asked only for an otherwise-pushable block. Carrying also requires
    /// a live owned payload; raw-only or currently borrowed native payloads remain refused.
    /// Diverges from vanilla: push only never discards a block entity.
    #[hook(
        side = "both",
        contract = "replace",
        out = "reaction",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.getPistonPushReaction"
    )]
    fn piston_push_reaction();
}
