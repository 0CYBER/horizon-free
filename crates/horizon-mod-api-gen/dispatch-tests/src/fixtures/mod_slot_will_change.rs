/// A pre-change listener: records the slot and cancels a named one.
struct Pre(Arc<Mutex<Vec<String>>>);

#[mod_api::__subscriptions]
impl Hooks for Pre {
    fn mod_slot_will_change(&self, _: &dyn Host, event: &mut ModSlotWillChangeEvent<'_>) -> Flow {
        self.0.lock().unwrap().push(event.slot.to_owned());
        if event.slot == "ring" {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[test]
fn pre_change_listener_can_cancel_and_sees_the_fields() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    install(vec![ModEntry::new("pre", Box::new(Pre(heard.clone())))].into());
    let stack = ItemStack {
        item: "fixture:ring".into(),
        count: 1,
    };
    let mut cancel = false;
    dispatch_mod_slot_will_change(
        &TestHost,
        calls::ModSlotWillChangeCall {
            holder: EntityRef,
            slot: "ring",
            index: 2,
            cosmetic: false,
            old: None,
            new: Some(&stack),
            source: "equip",
            cancel: &mut cancel,
        },
    );
    assert!(cancel);
    assert_eq!(*heard.lock().unwrap(), ["ring".to_owned()]);
    install(Vec::<ModEntry>::new().into());
}
