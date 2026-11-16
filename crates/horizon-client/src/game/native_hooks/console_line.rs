use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A line the server logged: its `level`, the `thread` it names and the
    /// `message` without time or thread. Server-side.
    pub(in crate::game) fn native_console_line(
        &mut self,
        level: &str,
        thread: &str,
        message: &str,
    ) {
        self.fire_console_line(level, thread, message);
    }
}
