/// The reads the signal-reading glass recorded on this thread, taken.
pub fn signal_reads() -> Vec<SignalRead> {
    SIGNAL_READS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}
