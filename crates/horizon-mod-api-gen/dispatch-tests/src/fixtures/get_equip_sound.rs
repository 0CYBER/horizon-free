/// An equip-sound part: records the stack, slot and sound it was given, then answers its own.
struct EquipSoundProbe(&'static str, Arc<Mutex<Vec<(u32, String, bool)>>>);

impl ItemPart for EquipSoundProbe {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::GetEquipSound)]
    }
    fn get_equip_sound(&self, _: &dyn Host, event: &mut GetEquipSoundEvent<'_>) {
        self.1.lock().unwrap().push((
            event.stack.count(),
            event.slot.to_owned(),
            event.sound.is_some(),
        ));
        if event.slot == self.0 {
            *event.sound = Some(Identifier);
        }
    }
}

#[test]
fn equip_sound_parts_start_from_the_stacks_answer_and_answer_in_turn() {
    install(Vec::<ModEntry>::new().into());
    let heard = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        items: ["other:slot", "curios:ring"]
            .map(|answer| leak(EquipSoundProbe(answer, heard.clone())) as &dyn ItemPart)
            .into(),
        ..Default::default()
    });
    let stack = ItemStack {
        item: "fixture:ring".into(),
        count: 1,
    };
    let mut sound = None;
    dispatch_get_equip_sound(&TestHost, &stack, "curios:ring", &mut sound);
    // The second part answers for this slot; the first leaves the stack's sound alone.
    assert!(sound.is_some());
    assert_eq!(
        *heard.lock().unwrap(),
        [
            (1, "curios:ring".to_owned(), false),
            (1, "curios:ring".to_owned(), false)
        ]
    );
    install_parts(Parts::default());
}
