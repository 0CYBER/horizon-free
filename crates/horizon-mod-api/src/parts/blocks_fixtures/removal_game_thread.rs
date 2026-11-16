/// Install a part on glass that asks, when a glass block is removed, for every neighbour to
/// be told, as an override of `affectNeighborsAfterRemoval` calling `updateNeighborsAt`.
pub fn removal_game_thread() {
    struct Leaver;
    #[mod_api::horizon_part]
    impl BlockPart for Leaver {
        fn affect_neighbors_after_removal(
            &self,
            _: &dyn LevelHost,
            event: &mut AffectNeighborsAfterRemovalEvent<'_>,
        ) {
            event.updates.push(NeighbourUpdate::Neighbours);
        }
    }
    struct LeaverMod;
    #[mod_api::__subscriptions]
    impl Hooks for LeaverMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let block_key = r.block_key("minecraft:glass");
            r.block_part(block_key, Leaver);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("leaver_test", Box::new(LeaverMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
