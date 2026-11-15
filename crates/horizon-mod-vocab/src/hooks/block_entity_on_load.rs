//! A mod block entity's first attachment to its logical level.

pub trait Hooks {
    /// The mod block entity `entity` has been added to its level, with saved
    /// data already applied, before its first tick (`IBlockEntityExtension.onLoad`).
    /// Fires once for each instance on both logical sides, including placement.
    /// A chunk loaded again creates new instances and calls this hook again.
    /// Read saved data through the attached handle and the supplied `HostSide`.
    #[notes = "`block_entity_on_load` is the per-instance attachment notice, after saved data is applied and before ticking. It also runs for fresh placement and for client chunk instances. The factory remains a separate creation step. Its counterpart is `IBlockEntityExtension.onLoad`."]
    #[hook(
        side = "both",
        contract = "listen",
        scope = "block_entity",
        key = "id",
        neoforge = "IBlockEntityExtension.onLoad"
    )]
    fn block_entity_on_load(id: BlockEntityTypeKey, entity: BlockEntityRef);
}
