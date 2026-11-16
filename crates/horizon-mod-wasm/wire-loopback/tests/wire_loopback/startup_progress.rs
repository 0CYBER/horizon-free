use super::*;
use std::sync::Mutex;

/// The game's screen: each call the guest made.
#[derive(Default)]
struct Record(Mutex<Vec<String>>);
impl Record {
    fn push(&self, call: String) {
        self.0.lock().unwrap().push(call);
    }
}
impl mod_api::StartupScreen for Record {
    fn open(&self, label: String, steps: u32) -> Result<u32, mod_api::StartupProgressError> {
        self.push(format!("open {label} {steps}"));
        Ok(9)
    }
    fn step(&self, bar: u32) {
        self.push(format!("step {bar}"));
    }
    fn set(&self, bar: u32, done: u32) {
        self.push(format!("set {bar} {done}"));
    }
    fn label(&self, bar: u32, label: String) {
        self.push(format!("label {bar} {label}"));
    }
    fn close(&self, bar: u32) {
        self.push(format!("close {bar}"));
    }
    fn message(&self, text: String) {
        self.push(format!("message {text}"));
    }
}

fn register(cx: &mod_api::RegistrationContext) {
    let bar = cx.progress("loopback", 10).unwrap();
    for _ in 0..4 {
        bar.step();
    }
    cx.message("loopback line");
}

#[test]
fn wire_loopback_a_guest_bar_reaches_the_game_screen_by_id() {
    static RECORD: std::sync::OnceLock<Arc<Record>> = std::sync::OnceLock::new();
    let record = RECORD.get_or_init(|| Arc::new(Record::default())).clone();
    // Repeated fixture installation retains the same observable screen.
    let _ = mod_api::__private::game::install_startup_screen(record.clone());
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct Fixture;
    impl Hooks for Fixture {
        fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
            &[]
        }
        fn register(&self, _: &mut dyn mod_api::Registrar, context: &mod_api::RegistrationContext) {
            let module = WasmMod {
                group: 0,
                subscribed: &[],
                listeners: &[],
                corrupt: false,
            };
            let mut cx = Box::new(boundary::Cx::default());
            cx.module = &module;
            cx.frames.push(boundary::Frame::default());
            let pointer = &mut *cx as *mut boundary::Cx;
            // SAFETY: the module and context outlive the synchronous calls and scopes.
            let _resources = unsafe { boundary::resource_scope(pointer, true) };
            let mut out = Writer::new();
            out.put(context);
            // SAFETY: the fixture owns cx until the transport scope drops.
            let transport = unsafe { host_dispatch::transport(pointer) };
            let _transport = TransportScope(None);
            guest::set_transport(Some(transport));
            let _guest = guest::resource_scope();
            let context: mod_api::RegistrationContext = Reader::new(out.bytes()).take().unwrap();
            register(&context);
        }
    }
    mod_api::__private::install_parts(
        &[mod_api::ModEntry::new("startup_fixture", Box::new(Fixture))],
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    )
    .unwrap();
    // Tests running beside this one may register too: count calls, not their order.
    let calls = record.0.lock().unwrap().clone();
    let count = |call: &str| calls.iter().filter(|made| *made == call).count();
    assert!(count("open loopback 10") >= 1, "{calls:?}");
    assert!(count("step 9") >= 4, "{calls:?}");
    assert!(count("message loopback line") >= 1, "{calls:?}");
    assert!(count("close 9") >= 1, "{calls:?}");
}
