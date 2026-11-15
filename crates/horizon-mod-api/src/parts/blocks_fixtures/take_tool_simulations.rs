/// The `simulated` flags the tool parts were asked with on this thread since the last call
/// (or the fixture's install), in order.
pub fn take_tool_simulations() -> Vec<bool> {
    TOOL_SIMULATIONS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}
