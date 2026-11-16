/// A highlight-tip part: records that it was asked, then writes its own name.
struct TipProbe(u32, Arc<Mutex<Vec<u32>>>);

impl ItemPart for TipProbe {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::HighlightTip)]
    }
    fn highlight_tip(&self, _: &dyn ClientHost, event: &mut HighlightTipEvent<'_>) {
        assert_eq!(event.stack.count(), 3);
        self.1.lock().unwrap().push(self.0);
        *event.name = Text;
    }
}

#[test]
fn highlight_tip_parts_are_asked_in_turn() {
    install(Vec::<ModEntry>::new().into());
    let heard = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        items: [1, 2]
            .map(|id| leak(TipProbe(id, heard.clone())) as &dyn ItemPart)
            .into(),
        ..Default::default()
    });
    let stack = ItemStack {
        item: "fixture:stick".into(),
        count: 3,
    };
    let mut name = Text;
    dispatch_highlight_tip(&TestHost, &stack, &mut name);
    assert_eq!(*heard.lock().unwrap(), [1, 2]);
    install_parts(Parts::default());
}
