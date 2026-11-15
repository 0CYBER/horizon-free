//! One logger for this test binary, with thread-local diagnostic capture.
use std::cell::RefCell;
thread_local! { pub(crate) static RECORDS: RefCell<Vec<(log::Level, String)>> = const { RefCell::new(Vec::new()) }; }
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
pub(crate) fn clear() {
    RECORDS.with_borrow_mut(Vec::clear);
}
pub(crate) fn records() -> Vec<(log::Level, String)> {
    RECORDS.with_borrow(Clone::clone)
}
