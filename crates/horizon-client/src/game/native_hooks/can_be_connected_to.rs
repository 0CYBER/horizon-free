use super::*;

impl GameLevel<'_> {
    /// `IBlockExtension.canBeConnectedTo`: the parts of the block `state` at `pos` may answer
    /// whether the fence, wall, or bars or pane `connector` beside it (its rule named by
    /// `kind`) connects to it, `direction` pointing from the block toward the connector and
    /// `face_solid` whether the block's face toward it is sturdy. The level is read-only.
    #[inline(always)]
    pub(in crate::game) fn native_can_be_connected_to(
        &self,
        call: super::fire::CanBeConnectedToCall<'_>,
    ) {
        self.fire_can_be_connected_to(call);
    }

    /// `LevelAccess::adjust_can_be_connected_to`: on an authoritative server, what the parts
    /// of `state` answer once the connector's vanilla rule answered `vanilla`. A panicking
    /// part is contained with a warning and vanilla's answer is kept.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn can_be_connected_to(
        &self,
        kind: ConnectionKind,
        pos: BlockPos,
        state: BlockStateId,
        connector: BlockStateId,
        direction: Direction,
        face_solid: bool,
        vanilla: bool,
    ) -> bool {
        if !self.native_authoritative {
            return vanilla;
        }
        let mut answer = None;
        if let Err(error) = horizon_client::crash::contain(|| {
            self.native_can_be_connected_to(super::fire::CanBeConnectedToCall {
                dimension: self.dimension,
                pos,
                state,
                connector,
                kind,
                direction,
                face_solid,
                answer: &mut answer,
            });
        }) {
            log::warn!("can_be_connected_to callback failed: {error}");
            return vanilla;
        }
        answer.unwrap_or(vanilla)
    }
}
