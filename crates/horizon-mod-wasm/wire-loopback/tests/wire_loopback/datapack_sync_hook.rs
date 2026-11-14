pub(super) struct DatapackSyncHooks;
impl DatapackSyncHooks {
    pub(super) fn datapack_sync(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::DatapackSyncEvent<'_>,
    ) {
        if event.player.uuid() == 12 {
            event.recipe_types.push("testmod:grinding".into());
        }
    }
}
