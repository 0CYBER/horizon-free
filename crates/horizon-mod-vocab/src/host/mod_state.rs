//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `mod_state` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait ModStateHost {
    /// Side: both.
    /// Capture a SNAPSHOT of the current dimension's attachment residency and values.
    /// Later changes require a fresh query. Removed-but-unswept entities may appear;
    /// writes through stale holders fail. Values are neither cloned nor eagerly decoded.
    #[access(WorldOnly)]
    fn attachment_query(&self, namespace: &str, name: &str) -> AttachmentBatch;
    /// Side: both.
    /// The server-owned holders' attachments; `TYPE.get(host, holder)` reads
    /// one. On a client it is the copy of what the server synced: `set` and
    /// `remove` change only the copy.
    #[access(WorldOnly)]
    fn attachments(&mut self) -> &mut dyn AttachmentStore;
}
/// The server host's methods of the `mod_state` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ModStateServerHost {
    /// Side: server.
    /// Validate captured residency against the current dimension immediately before
    /// an ordinary journaled attachment write. This does not open a transaction.
    #[access(WorldOnly)]
    fn attachment_validate_write(
        &self,
        token: AttachmentWriteToken,
    ) -> Result<(), AttachmentRowError>;
    /// Side: server.
    /// Opens a transaction, nested in the open one if any (`Transaction.open`), and gives back
    /// the seal that ends it. `host.transaction` calls it; use that instead.
    #[access(WorldOnly)]
    fn transaction_open(&mut self) -> TransactionSeal;
    /// Side: server.
    /// Closes the transaction `seal` ends, committed or aborted, aborting first any opened
    /// inside it and left open. Refused when `seal` names no open transaction, so a mod ends
    /// only a transaction it opened. `host.transaction` calls it.
    #[access(WorldOnly)]
    fn transaction_close(&mut self, seal: &TransactionSeal, commit: bool);
    /// Side: server.
    /// Where the transaction system is (`Transaction.getLifecycle`): none open, one open, a
    /// nested one closing, or the outermost closing while the game tells the world of a commit.
    #[access(WorldOnly)]
    fn transaction_lifecycle(&self) -> TransactionLifecycle;
    /// Side: server.
    /// The innermost open transaction, to write in (`Transaction.getCurrentOpenedTransaction`);
    /// none when none is open. It names the transaction only: ending it still takes the seal its
    /// opener holds.
    #[access(WorldOnly)]
    fn transaction_current(&self) -> Option<Transaction>;
    /// Side: server.
    /// How deeply `tx` is nested (`TransactionContext.depth`): 0 for the outermost, none for a
    /// transaction that is not open.
    #[access(WorldOnly)]
    fn transaction_depth(&self, tx: &Transaction) -> Option<u32>;
    /// Side: server.
    /// Asks for `Hooks::scheduled_task` to be called with `token` after `delay_ticks` server
    /// ticks (`MinecraftServer.tell(new TickTask(getTickCount() + delay, ..))`; 0 and 1 both
    /// mean the next tick, and a paused server does not advance it). Tasks due together run in
    /// the order they were scheduled. Only inside a mod's hook; the task belongs to that mod and
    /// is dropped with the world.
    #[access(WorldOnly)]
    fn schedule_task(&mut self, delay_ticks: u32, token: u64) -> Result<TaskId, ScheduleError>;
    /// Side: server.
    /// Withdraws a task the calling mod scheduled: true if it was still waiting.
    #[access(WorldOnly)]
    fn cancel_task(&mut self, id: TaskId) -> bool;
}
