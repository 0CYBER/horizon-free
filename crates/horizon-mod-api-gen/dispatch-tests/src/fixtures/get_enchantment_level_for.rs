/// A level mod: records the owner and place it was asked about.
struct OwnerProbe(Arc<Mutex<Vec<(bool, Option<String>)>>>);

#[mod_api::__subscriptions]
impl Hooks for OwnerProbe {
    fn get_enchantment_level_for(&self, _: &dyn Host, event: &mut GetEnchantmentLevelForEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push((event.owner.is_some(), event.mod_slot.map(str::to_owned)));
    }
}

#[test]
fn enchantment_level_owner_and_place_reach_the_hook() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    install(vec![ModEntry::new("owner", Box::new(OwnerProbe(heard.clone())))].into());
    let stack = ItemStack {
        item: "fixture:ring".into(),
        count: 1,
    };
    let mut enchantments = ItemEnchantments;
    dispatch_get_enchantment_level_for(
        &TestHost,
        &stack,
        None,
        Some(EntityRef),
        Some("curios:ring"),
        &mut enchantments,
    );
    dispatch_get_enchantment_level_for(&TestHost, &stack, None, None, None, &mut enchantments);
    assert_eq!(
        *heard.lock().unwrap(),
        [(true, Some("curios:ring".to_owned())), (false, None)]
    );
    install(Vec::<ModEntry>::new().into());
}
