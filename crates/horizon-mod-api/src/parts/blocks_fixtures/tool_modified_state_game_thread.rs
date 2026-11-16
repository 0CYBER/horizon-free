/// Install parts that decide what a tool makes of a block: an axe on an oak log makes a
/// stripped spruce log (a replaced answer), a birch log and a campfire are left alone (denied), and glass,
/// which vanilla leaves alone, strips to a stripped oak log (allowed).
pub fn tool_modified_state_game_thread() {
    struct Tool(Option<&'static str>);
    #[mod_api::horizon_part]
    impl BlockPart for Tool {
        fn tool_modified_state(
            &self,
            host: &dyn Host,
            event: &mut ToolModifiedStateEvent<'_>,
        ) -> Flow {
            TOOL_SIMULATIONS.with(|r| r.borrow_mut().push(event.tool.simulated));
            match self.0 {
                Some(result) => {
                    *event.final_state = host.block_state_id(result);
                    Flow::Continue
                }
                None => Flow::Cancel,
            }
        }
    }
    struct ToolMod;
    #[mod_api::__subscriptions]
    impl Hooks for ToolMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            for (block, result) in [
                ("minecraft:oak_log", Some("minecraft:stripped_spruce_log")),
                ("minecraft:birch_log", None),
                ("minecraft:campfire", None),
                ("minecraft:glass", Some("minecraft:stripped_oak_log")),
            ] {
                let key = r.block_key(block);
                r.block_part(key, Tool(result));
            }
        }
    }
    TOOL_SIMULATIONS.with(|r| r.borrow_mut().clear());
    let mods: crate::Mods = vec![ModEntry::new("tool_state_test", Box::new(ToolMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}

std::thread_local! {
    static DESTROYS: std::cell::RefCell<Vec<(bool, u32)>> = const { std::cell::RefCell::new(Vec::new()) };
}
