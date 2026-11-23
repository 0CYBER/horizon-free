//! The one logger this crate's tests share: `log` takes a single logger per process, and the
//! tests that read what was logged run side by side, so each reads only its own thread's lines.
use std::cell::RefCell;

thread_local! {
    static RECORDS: RefCell<Vec<(log::Level, String)>> = const { RefCell::new(Vec::new()) };
}

struct Capture;
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        RECORDS
            .with_borrow_mut(|records| records.push((record.level(), record.args().to_string())));
    }
    fn flush(&self) {}
}

static CAPTURE: Capture = Capture;
static INSTALL: std::sync::Once = std::sync::Once::new();

/// Installs the logger (once per process, errors and worse) and empties this thread's lines.
pub(crate) fn capture() {
    INSTALL.call_once(|| {
        log::set_logger(&CAPTURE).expect("only test_log installs a logger");
        log::set_max_level(log::LevelFilter::Error);
    });
    clear();
}

/// Forgets this thread's lines.
pub(crate) fn clear() {
    RECORDS.with_borrow_mut(Vec::clear);
}

/// `f` given what this thread logged since [`capture`] or [`clear`].
pub(crate) fn with_records<R>(f: impl FnOnce(&Vec<(log::Level, String)>) -> R) -> R {
    RECORDS.with_borrow(f)
}
