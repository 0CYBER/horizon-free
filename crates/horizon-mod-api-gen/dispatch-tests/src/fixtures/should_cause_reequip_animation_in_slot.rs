/// A slot-aware reequip mod: records the slot and the answer it arrived with, then answers its own.
struct SlotReequipProbe(bool, Arc<Mutex<Vec<(String, bool)>>>);

#[mod_api::__subscriptions]
impl Hooks for SlotReequipProbe {
    fn should_cause_reequip_animation_in_slot(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldCauseReequipAnimationInSlotEvent<'_>,
    ) {
        self.1
            .lock()
            .unwrap()
            .push((event.slot.to_owned(), event.reequip));
        event.reequip = self.0;
    }
}

#[test]
fn slot_aware_reequip_mods_hear_the_slot_and_answer_in_turn() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    install(
        vec![
            ModEntry::new("first", Box::new(SlotReequipProbe(true, heard.clone()))),
            ModEntry::new("second", Box::new(SlotReequipProbe(false, heard.clone()))),
        ]
        .into(),
    );
    let stick = |count| ItemStack {
        item: "fixture:stick".into(),
        count,
    };
    let mut reequip = false;
    dispatch_should_cause_reequip_animation_in_slot(
        &TestHost,
        &stick(2),
        Some(&stick(1)),
        Hand::Off,
        "offhand",
        false,
        &mut reequip,
    );
    // The second mod is given the first mod's answer, and its own is the last word.
    assert!(!reequip);
    assert_eq!(
        *heard.lock().unwrap(),
        [("offhand".to_owned(), false), ("offhand".to_owned(), true)]
    );
    install(Vec::<ModEntry>::new().into());
}
