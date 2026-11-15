/// Install a connection rule on oak fences, cobblestone walls and glass panes for real-game
/// tests: it flips vanilla's answer.
pub fn connection_game_thread() {
    struct ConnectionMod;
    #[mod_api::__subscriptions]
    impl Hooks for ConnectionMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            for block in [
                "minecraft:oak_fence",
                "minecraft:cobblestone_wall",
                "minecraft:glass_pane",
            ] {
                let block_key = r.block_key(block);
                r.block_part(block_key, ConnectionRule);
            }
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("connection_test", Box::new(ConnectionMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}

std::thread_local! {
    static SIGNAL_READS: std::cell::RefCell<Vec<SignalRead>> = const { std::cell::RefCell::new(Vec::new()) };
}
