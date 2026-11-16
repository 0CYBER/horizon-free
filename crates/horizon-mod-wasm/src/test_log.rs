//! Shared process-wide log capture for unit tests.
pub(crate) struct Capture(pub(crate) std::sync::Mutex<Vec<String>>);
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        self.0.lock().unwrap().push(record.args().to_string());
    }
    fn flush(&self) {}
}
pub(crate) static CAPTURE: Capture = Capture(std::sync::Mutex::new(Vec::new()));
pub fn capture_logs() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        log::set_logger(&CAPTURE).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
    });
}

/// Take records emitted since the previous observation.
pub fn take_logs() -> Vec<String> {
    std::mem::take(&mut *CAPTURE.0.lock().unwrap())
}
