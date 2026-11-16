//! Thread-local warning capture shared by tests in this crate.
use std::cell::RefCell;
thread_local! { static WARNINGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) }; }
struct Logger;
impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if record.level() == log::Level::Warn {
            WARNINGS.with(|lines| lines.borrow_mut().push(record.args().to_string()));
        }
    }
    fn flush(&self) {}
}
pub(crate) fn capture<T>(run: impl FnOnce() -> T) -> (T, Vec<String>) {
    static LOGGER: Logger = Logger;
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        log::set_logger(&LOGGER).expect("test warning logger");
        log::set_max_level(log::LevelFilter::Warn);
    });
    WARNINGS.with(|lines| lines.borrow_mut().clear());
    let result = run();
    (result, WARNINGS.with(|lines| lines.take()))
}
