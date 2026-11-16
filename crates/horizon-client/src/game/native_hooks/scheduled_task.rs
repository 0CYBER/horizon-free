use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A task a mod scheduled (`schedule_task`) is due: at the start of the server tick it was
    /// due on, after `native_server_tick`'s start phase. `owner` is the scheduling mod's id.
    pub(in crate::game) fn native_scheduled_task(&mut self, id: TaskId, owner: &str, token: u64) {
        self.fire_scheduled_task(id, owner, token);
    }
}
