pub fn friction_game_thread() -> Arc<std::sync::atomic::AtomicUsize> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    struct Answer(f32, Option<&'static str>, Arc<AtomicUsize>);
    #[mod_api::horizon_part]
    impl BlockPart for Answer {
        fn friction(&self, _: &dyn LoopContext, event: &mut FrictionEvent<'_>) {
            self.2.fetch_add(1, Ordering::SeqCst);
            if self.1.is_none_or(|kind| kind == event.kind) {
                event.friction = self.0;
            }
        }
    }
    struct FrictionMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for FrictionMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            for (block, answer) in [
                (
                    "minecraft:glass",
                    Answer(0.98, Some("minecraft:player"), self.0.clone()),
                ),
                ("minecraft:white_wool", Answer(0.98, None, self.0.clone())),
                ("minecraft:red_wool", Answer(f32::NAN, None, self.0.clone())),
                ("minecraft:blue_wool", Answer(3.0, None, self.0.clone())),
                (
                    "minecraft:green_wool",
                    Answer(f32::INFINITY, None, self.0.clone()),
                ),
                ("minecraft:black_wool", Answer(-0.5, None, self.0.clone())),
            ] {
                let key = r.block_key(block);
                r.block_part(key, answer);
            }
        }
    }
    let mods: crate::Mods = vec![ModEntry::new(
        "friction_test",
        Box::new(FrictionMod(calls.clone())),
    )]
    .into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
    calls
}
