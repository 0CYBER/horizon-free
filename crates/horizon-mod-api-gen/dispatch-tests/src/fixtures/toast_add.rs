#[test]
fn toast_add_passes_kind_and_text_edits_and_stops_on_cancel() {
    struct Filter(bool);
    #[mod_api::__subscriptions]
    impl Hooks for Filter {
        fn toast_add(&self, _: &dyn ClientHost, event: &mut ToastAddEvent<'_>) -> Flow {
            assert_eq!(event.kind, generated::mod_api::ToastKind::Advancement);
            if self.0 {
                assert!(event.description.is_some());
                *event.description = None;
                Flow::Cancel
            } else {
                assert!(event.description.is_none());
                *event.description = Some(Text);
                Flow::Continue
            }
        }
    }
    install(
        vec![
            ModEntry::new("editor", Box::new(Filter(false))),
            ModEntry::new("cancel", Box::new(Filter(true))),
            ModEntry::new("late", Box::new(Filter(false))),
        ]
        .into(),
    );
    ENTERED.with(|entered| entered.borrow_mut().clear());
    let mut description = None;
    let mut cancel = false;
    dispatch_toast_add(
        &TestHost,
        generated::mod_api::ToastKind::Advancement,
        &mut Text,
        &mut description,
        &mut cancel,
    );
    assert!(cancel);
    assert!(description.is_none());
    ENTERED.with(|entered| {
        assert_eq!(
            entered.borrow().as_slice(),
            [(0, "toast_add"), (1, "toast_add")]
        );
    });
}

mod api {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ToastKind {
        Advancement,
    }
}
