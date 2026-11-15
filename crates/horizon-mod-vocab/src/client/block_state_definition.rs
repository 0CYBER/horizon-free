//! What a mod's block-state definition type answers (`instantiate_block_state_definition`).

/// One answer a block-state definition type gives for a blockstate file naming it
/// (`register_block_state_models`): the model one of the block's states draws, or a refusal of
/// the whole file. A state no `Model` names inherits the lower pack layer's selection
/// when present; otherwise it draws no model.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: mods construct picks and the game matches every kind
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockStateModelPick {
    /// One state draws one model, as a blockstate variant names it.
    Model {
        /// The state: an index into the event's `states`.
        state: u32,
        /// The model (`mymod:block/lamp_on`).
        model: String,
        /// Quarter turns about X, Y and Z (a variant's `x`, `y` and `z` over 90), taken modulo
        /// four.
        turns: [u8; 3],
        /// Whether the textures stay still as the model turns (a variant's `uvlock`).
        uvlock: bool,
    },
    /// The file doesn't read as this type: it is refused whole, as a vanilla definition that
    /// doesn't read is.
    Refuse {
        /// Why, logged with the file's name.
        reason: String,
    },
}
