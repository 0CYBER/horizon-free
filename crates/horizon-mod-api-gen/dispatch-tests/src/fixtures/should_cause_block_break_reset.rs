/// A break-reset part: records the stacks and answer it was given, then answers its own.
struct ResetProbe(bool, Arc<Mutex<Vec<(u32, Option<u32>, bool)>>>);

impl ItemPart for ResetProbe {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::ShouldCauseBlockBreakReset)]
    }
    fn should_cause_block_break_reset(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldCauseBlockBreakResetEvent<'_>,
    ) {
        self.1.lock().unwrap().push((
            event.stack.count(),
            event.held.map(ItemStack::count),
            event.reset,
        ));
        event.reset = self.0;
    }
}

#[test]
fn break_reset_parts_start_from_vanillas_answer_and_answer_in_turn() {
    install(Vec::<ModEntry>::new().into());
    let heard = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        items: [false, true]
            .map(|answer| leak(ResetProbe(answer, heard.clone())) as &dyn ItemPart)
            .into(),
        ..Default::default()
    });
    let began = ItemStack {
        item: "fixture:stick".into(),
        count: 1,
    };
    let mut reset = true;
    dispatch_should_cause_block_break_reset(&TestHost, &began, None, &mut reset);
    assert!(reset);
    assert_eq!(*heard.lock().unwrap(), [(1, None, true), (1, None, false)]);
    install_parts(Parts::default());
}
