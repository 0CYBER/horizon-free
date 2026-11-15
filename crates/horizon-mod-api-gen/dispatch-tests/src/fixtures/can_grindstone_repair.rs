//! The `can_grindstone_repair` fixture: the first `ItemPart` method with `context = "loop"`.
//! The generated method takes the loop context and no `Host`, its key is the `&ItemStack`,
//! and the dispatch looks the parts up by the stack's item (`item_listens`'s table), each
//! starting from the answer before it.

/// A grindstone part: records the side, the stack's count and the answer it was given, then
/// answers its own.
struct GrindstoneProbe(bool, Arc<Mutex<Vec<(&'static str, u32, bool)>>>);

impl ItemPart for GrindstoneProbe {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::CanGrindstoneRepair)]
    }
    fn can_grindstone_repair(
        &self,
        context: &dyn LoopContext,
        event: &mut CanGrindstoneRepairEvent<'_>,
    ) {
        self.1
            .lock()
            .unwrap()
            .push((context.side(), event.stack.count, event.can_repair));
        event.can_repair = self.0;
    }
}

#[test]
fn item_loop_parts_get_the_context_and_start_from_vanillas_answer() {
    install(Vec::<ModEntry>::new().into());
    let heard = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        items: [true, false]
            .map(|answer| leak(GrindstoneProbe(answer, heard.clone())) as &dyn ItemPart)
            .into(),
        ..Default::default()
    });
    let stick = ItemStack {
        item: "fixture:stick".into(),
        count: 2,
    };
    for vanilla in [false, true] {
        let mut can_repair = vanilla;
        dispatch_can_grindstone_repair(&Loop("server"), &stick, &mut can_repair);
        // The second part is given the first part's answer, and its own is the last word.
        assert!(!can_repair);
        assert_eq!(
            std::mem::take(&mut *heard.lock().unwrap()),
            [("server", 2, vanilla), ("server", 2, true)]
        );
    }
    // With no part installed the answer stays vanilla's.
    install_parts(Parts::default());
    let mut can_repair = true;
    dispatch_can_grindstone_repair(&Loop("client"), &stick, &mut can_repair);
    assert!(can_repair);
    assert!(heard.lock().unwrap().is_empty());
}
