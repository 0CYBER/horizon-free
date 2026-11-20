/// A level mod on the original hook: records the reads the engine still sends there.
struct LegacyLevels(Arc<Mutex<Vec<(u32, bool)>>>);

#[mod_api::__subscriptions]
impl Hooks for LegacyLevels {
    fn get_enchantment_level(&self, event: &mut GetEnchantmentLevelEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push((event.stack.count(), event.target.is_some()));
    }
}

#[test]
fn ordinary_stack_reads_still_reach_the_original_hook() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    install(
        vec![ModEntry::new(
            "legacy",
            Box::new(LegacyLevels(heard.clone())),
        )]
        .into(),
    );
    let stack = ItemStack {
        item: "fixture:ring".into(),
        count: 1,
    };
    let mut enchantments = ItemEnchantments;
    dispatch_get_enchantment_level(&stack, None, &mut enchantments);
    dispatch_get_enchantment_level(&stack, Some(&Identifier), &mut enchantments);
    assert_eq!(*heard.lock().unwrap(), [(1, false), (1, true)]);
    install(Vec::<ModEntry>::new().into());
}
