/// For each server-side harvest read of the witness part's block (stone) on this thread since
/// the last call, in order: whether the block was still in the world when it was read.
pub fn take_harvest_reads() -> Vec<bool> {
    HARVEST_READS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

std::thread_local! {
    static POT_READS: std::cell::RefCell<Vec<bool>> = const { std::cell::RefCell::new(Vec::new()) };
}
