//! A tool acting on a block (an axe strips, a shovel flattens or douses, a hoe tills): the
//! block's parts say what the block becomes, then the global hook hears the result.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    /// What the tool action `tool` makes of the block `block`: `state` starts with vanilla's
    /// answer. The block's parts answer first (`tool_modified_state`,
    /// `IBlockExtension.getToolModifiedState`), and `block_tool_modification` hears their result
    /// unless they cancelled the action.
    pub(in crate::game) fn mod_tool_modification(
        &self,
        stack: &ItemStack,
        block: BlockStateId,
        tool: &mod_api::ToolUse,
        state: &mut Option<BlockStateId>,
        cancel: &mut bool,
    ) {
        self.native_tool_modified_state(block, stack, tool, state, cancel);
        if !*cancel {
            self.native_block_tool_modification(stack, tool, state, cancel);
        }
    }
}
