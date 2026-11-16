#[cfg(test)]
/// State 1 is a `test:gauge`, state 2 a `test:other`.
pub(crate) struct Level;
#[cfg(test)]
impl LevelHost for Level {
    fn block_state(&self, _: [i32; 3]) -> Option<crate::BlockState> {
        None
    }
    fn state_with_property(&self, _: BlockState, _: &str, _: &str) -> Option<BlockState> {
        None
    }
    fn state_block(&self, state: BlockState) -> Option<&str> {
        let metadata = crate::__private::block_definitions()?;
        let slot = mod_vocab::__private::parts_installation::block_key_slot(state.block()) as usize;
        if u32::from(state.ordinal()) >= metadata.definitions.get(slot)?.state_count() {
            return None;
        }
        metadata
            .names
            .entries
            .get(slot)
            .map(|entry| entry.id.as_str())
    }
}
