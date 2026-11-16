/// Install parts that change what a path may cross: glass becomes land and air, air
/// stops being land and becomes water.
pub fn pathfindable_game_thread() {
    struct Answers(&'static [(PathComputationType, bool)]);
    #[mod_api::horizon_part]
    impl BlockPart for Answers {
        fn is_pathfindable(&self, _: &dyn Host, event: &mut IsPathfindableEvent) {
            if let Some((_, answer)) = self.0.iter().find(|(kind, _)| *kind == event.kind) {
                event.pathfindable = *answer;
            }
        }
    }
    struct PathMod;
    #[mod_api::__subscriptions]
    impl Hooks for PathMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let block_key = r.block_key("minecraft:glass");
            r.block_part(
                block_key,
                Answers(&[
                    (PathComputationType::Land, true),
                    (PathComputationType::Air, true),
                ]),
            );
            let block_key = r.block_key("minecraft:air");
            r.block_part(
                block_key,
                Answers(&[
                    (PathComputationType::Land, false),
                    (PathComputationType::Water, true),
                ]),
            );
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("path_test", Box::new(PathMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
