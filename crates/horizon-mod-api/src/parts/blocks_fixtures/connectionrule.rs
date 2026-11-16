/// Makes a fence, wall or pane flip vanilla's answer toward the east and refuse every other
/// side.
pub(crate) struct ConnectionRule;
#[mod_api::horizon_part]
impl BlockPart for ConnectionRule {
    fn connects_to(&self, _: &dyn LevelHost, event: &mut ConnectsToEvent) {
        event.connected = event.direction == Direction::East && !event.connected;
    }
}
