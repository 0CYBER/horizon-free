/// A committed listener: records the source and slot it was told.
struct Changed(Arc<Mutex<Vec<(String, String)>>>);

#[mod_api::__subscriptions]
impl Hooks for Changed {
    fn mod_slot_changed(&self, _: &dyn Host, event: &mut ModSlotChangedEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push((event.slot.to_owned(), event.source.to_owned()));
    }
}

#[test]
fn committed_listener_sees_the_slot_and_source() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    install(vec![ModEntry::new("changed", Box::new(Changed(heard.clone())))].into());
    let stack = ItemStack {
        item: "fixture:ring".into(),
        count: 1,
    };
    dispatch_mod_slot_changed(
        &TestHost,
        calls::ModSlotChangedCall {
            holder: EntityRef,
            slot: "ring",
            index: 2,
            cosmetic: true,
            old: None,
            new: Some(&stack),
            source: "equip",
        },
    );
    assert_eq!(
        *heard.lock().unwrap(),
        [("ring".to_owned(), "equip".to_owned())]
    );
    install(Vec::<ModEntry>::new().into());
}
