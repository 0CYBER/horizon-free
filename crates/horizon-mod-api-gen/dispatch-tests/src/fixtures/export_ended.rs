mod stubs {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum ExportState {
        Finished,
        Failed,
    }
}

#[test]
fn export_ended_tells_listeners_the_request_and_written_frames() {
    type Endings = Arc<Mutex<Vec<(u32, ExportState, u32)>>>;
    struct Listener(Endings);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn export_ended(&self, _: &mut dyn ClientHost, event: &mut ExportEndedEvent) {
            self.0
                .lock()
                .unwrap()
                .push((event.request, event.state, event.frames));
        }
    }

    let seen = Endings::default();
    install(vec![ModEntry::new("capture", Box::new(Listener(seen.clone())))].into());
    dispatch_export_ended(&mut TestHost, 4, ExportState::Finished, 32);
    dispatch_export_ended(&mut TestHost, 5, ExportState::Failed, 1);
    assert_eq!(
        *seen.lock().unwrap(),
        [(4, ExportState::Finished, 32), (5, ExportState::Failed, 1)]
    );
}
