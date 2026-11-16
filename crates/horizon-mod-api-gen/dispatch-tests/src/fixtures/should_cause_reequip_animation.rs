/// A reequip part: records the stacks, hand and answer it was given, then answers its own.
struct ReequipProbe(bool, Arc<Mutex<Vec<(u32, Option<u32>, bool, bool)>>>);

impl ItemPart for ReequipProbe {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::ShouldCauseReequipAnimation)]
    }
    fn should_cause_reequip_animation(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldCauseReequipAnimationEvent<'_>,
    ) {
        assert!(matches!(event.hand, Hand::Main));
        self.1.lock().unwrap().push((
            event.stack.count(),
            event.old.map(ItemStack::count),
            event.slot_changed,
            event.reequip,
        ));
        event.reequip = self.0;
    }
}

#[test]
fn reequip_parts_start_from_vanillas_answer_and_answer_in_turn() {
    install(Vec::<ModEntry>::new().into());
    let heard = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        items: [true, false]
            .map(|answer| leak(ReequipProbe(answer, heard.clone())) as &dyn ItemPart)
            .into(),
        ..Default::default()
    });
    let stick = |count| ItemStack {
        item: "fixture:stick".into(),
        count,
    };
    let mut reequip = true;
    dispatch_should_cause_reequip_animation(
        &TestHost,
        &stick(2),
        Some(&stick(1)),
        Hand::Main,
        true,
        &mut reequip,
    );
    // The second part is given the first part's answer, and its own is the last word.
    assert!(!reequip);
    assert_eq!(
        *heard.lock().unwrap(),
        [(2, Some(1), true, true), (2, Some(1), true, true)]
    );
    install_parts(Parts::default());
}
