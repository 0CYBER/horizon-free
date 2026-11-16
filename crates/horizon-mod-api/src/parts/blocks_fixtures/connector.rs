/// Connects redstone wire to its block from any side.
pub(crate) struct Connector;
#[mod_api::horizon_part]
impl BlockPart for Connector {
    fn should_redstone_wire_connect_to(
        &self,
        _: &dyn LevelHost,
        event: &mut ShouldRedstoneWireConnectToEvent,
    ) {
        event.connected = true;
    }

    fn connects_to(&self, _: &dyn LevelHost, event: &mut ConnectsToEvent) {
        event.connected = event.kind == ConnectionKind::Wall;
    }
}
