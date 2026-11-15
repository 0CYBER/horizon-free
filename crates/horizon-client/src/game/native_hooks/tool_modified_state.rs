use super::*;

impl Game<'_> {
    /// The state the block `state` becomes under the tool action `tool` of the
    /// held `stack` (`IBlockExtension.getToolModifiedState`): `final_state`
    /// starts with vanilla's answer and the block's parts may allow, replace
    /// or, with `cancel`, deny it. Asked before the global
    /// `block_tool_modification`.
    #[inline(always)]
    pub(in crate::game) fn native_tool_modified_state(
        &self,
        state: BlockStateId,
        stack: &ItemStack,
        tool: &ToolUse,
        final_state: &mut Option<BlockStateId>,
        cancel: &mut bool,
    ) {
        self.fire_tool_modified_state(state, stack, tool, final_state, cancel);
    }
}
