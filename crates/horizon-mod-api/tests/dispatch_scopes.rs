#![cfg(feature = "testing")]

use mod_api::testing::FakeHost;
use mod_api::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
};

struct State {
    phase: AtomicUsize,
    nested: AtomicBool,
    calls: [AtomicUsize; 2],
}
struct Probe {
    id: &'static str,
    index: usize,
    state: Arc<State>,
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        use mod_api::content::crash::current_native_call;
        let expected = mod_api::content::crash::ModCall {
            id: self.id,
            hook: "client_tick_pre",
            part: None,
        };
        assert_eq!(current_native_call(), Some(expected));
        self.state.calls[self.index].fetch_add(1, SeqCst);
        let phase = self.state.phase.load(SeqCst);
        if phase == 0 && self.index == 0 {
            // Enabling inside the first listener must not change the snapshot
            // already selected for this dispatch's remaining listeners.
            assert!(mod_api::__private::profiler::start());
        }
        if phase == 1 && self.index == 0 && !self.state.nested.swap(true, SeqCst) {
            let frame = mod_vocab::content::crash::native_frame();
            dispatch_client_tick_pre(host);
            assert_eq!(current_native_call(), Some(expected));
            assert_eq!(mod_vocab::content::crash::native_frame(), frame);
            self.state.nested.store(false, SeqCst);
        }
        assert!(!(phase == 2 && self.index == 1), "listener panic");
    }
}

#[test]
fn dispatch_snapshot_nested_scopes_and_unwind_restore_the_outer_owner() {
    mod_api::__private::initialize().unwrap();
    let state = Arc::new(State {
        phase: AtomicUsize::new(0),
        nested: AtomicBool::new(false),
        calls: [AtomicUsize::new(0), AtomicUsize::new(0)],
    });
    mod_api::__private::install(
        vec![
            ModEntry::new(
                "first",
                Box::new(Probe {
                    id: "first",
                    index: 0,
                    state: state.clone(),
                }),
            ),
            ModEntry::new(
                "second",
                Box::new(Probe {
                    id: "second",
                    index: 1,
                    state: state.clone(),
                }),
            ),
        ]
        .into(),
    );
    let outer_call = mod_api::content::crash::ModCall {
        id: "outer",
        hook: "host",
        part: None,
    };
    let outer = mod_api::content::crash::enter_native_mod(outer_call);
    let frame = mod_vocab::content::crash::native_frame();
    let mut host = FakeHost::new();
    dispatch_client_tick_pre(&mut host);
    mod_api::__private::profiler::stop();
    assert!(!mod_api::__private::profiler::summary().contains("client_tick_pre"));

    assert!(mod_api::__private::profiler::start());
    state.phase.store(1, SeqCst);
    dispatch_client_tick_pre(&mut host);
    assert_eq!(
        mod_api::content::crash::current_native_call(),
        Some(outer_call)
    );
    state.phase.store(2, SeqCst);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch_client_tick_pre(&mut host)
    }));
    assert!(panic.is_err());
    assert_eq!(
        mod_api::content::crash::current_native_call(),
        Some(outer_call)
    );
    assert_eq!(mod_vocab::content::crash::native_frame(), frame);
    mod_api::__private::profiler::stop();
    let report = mod_api::__private::profiler::summary();
    assert_eq!(
        report.matches("client_tick_pre [-]: count=3 ").count(),
        2,
        "{report}"
    );
    assert_eq!(
        state.calls.each_ref().map(|count| count.load(SeqCst)),
        [4, 4]
    );
    drop(outer);
    assert_eq!(mod_api::content::crash::current_native_call(), None);
}
