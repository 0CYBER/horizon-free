//! One logger for this probe's test binary, with thread-local diagnostic capture.
//!
//! `join_seq`'s tests read the diagnostics the residency table logs
//! (`residency/table/completion_tests.rs`), which `horizon-client` keeps in
//! `crate::test_log` (crates/horizon-client/src/test_log.rs). The probe imports the production
//! sources, so it carries the same shim, reduced to the two items those tests call.
use std::cell::RefCell;

thread_local! {
    pub(crate) static RECORDS: RefCell<Vec<(log::Level, String)>> = const { RefCell::new(Vec::new()) };
}

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        RECORDS.with_borrow_mut(|out| out.push((record.level(), record.args().to_string())));
    }

    fn flush(&self) {}
}

pub(crate) fn install() {
    static CAPTURE: Capture = Capture;
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        log::set_logger(&CAPTURE).expect("test diagnostic logger");
        log::set_max_level(log::LevelFilter::Warn);
    });
}
