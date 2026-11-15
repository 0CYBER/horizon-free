/// Install restitution parts on real blocks: glass records what it heard and answers 0.5
/// on a server's level and 0.25 on a client's; stone answers not a number.
pub fn restitution_game_thread() -> RestitutionHeard {
    struct Glass(RestitutionHeard);
    #[mod_api::horizon_part]
    impl BlockPart for Glass {
        fn bounce_restitution(&self, level: LevelSide<'_>, event: &mut BounceRestitutionEvent<'_>) {
            let (client, host) = match level {
                LevelSide::Server(host) => (false, host),
                LevelSide::Client(host) => (true, host),
            };
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            self.0.lock().unwrap().push((
                client,
                event.entity,
                event.kind.to_owned(),
                event.pos,
                host.block_state(pos) == Some(event.state),
            ));
            event.factor = if client { 0.25 } else { 0.5 };
        }
    }
    struct NotANumber;
    #[mod_api::horizon_part]
    impl BlockPart for NotANumber {
        fn bounce_restitution(&self, _: LevelSide<'_>, event: &mut BounceRestitutionEvent<'_>) {
            event.factor = f32::NAN;
        }
    }
    struct RestitutionMod(RestitutionHeard);
    #[mod_api::__subscriptions]
    impl Hooks for RestitutionMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            r.block_part(glass, Glass(self.0.clone()));
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, NotANumber);
        }
    }
    let heard = RestitutionHeard::default();
    let mods: crate::Mods = vec![ModEntry::new(
        "restitution_test",
        Box::new(RestitutionMod(heard.clone())),
    )]
    .into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
    heard
}
