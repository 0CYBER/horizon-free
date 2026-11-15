//! Client block entity synchronization notice.
pub trait Hooks {
    /// The synchronized `tag` has been merged into this client's block entity.
    /// `source` distinguishes chunk data from a later update packet. Empty chunk
    /// compounds send no notice; empty update-packet compounds do. Absent tags
    /// send no notice. Called once for each accepted tag,
    /// only for a matching mod block entity that the client created.
    /// Reads through `entity` already see the merged data. This notice cannot
    /// change the received tag. Missing tags and vanilla block entities call nothing.
    /// Related NeoForge receive paths `IBlockEntityExtension.onDataPacket` and
    /// `handleUpdateTag` perform loading; they are not counterparts of this notice,
    /// which runs after the merge.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (post-merge notice; IBlockEntityExtension.onDataPacket and handleUpdateTag are related loading paths, not counterparts)"
    )]
    fn block_entity_data_received();
}
