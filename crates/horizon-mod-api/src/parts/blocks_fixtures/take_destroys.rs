/// What the destroy parts were asked with on this thread since the last call (or the
/// fixture's install), in order: whether the break will harvest, and the size of the tool stack.
pub fn take_destroys() -> Vec<(bool, u32)> {
    DESTROYS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

std::thread_local! {
    static HARVEST_READS: std::cell::RefCell<Vec<bool>> = const { std::cell::RefCell::new(Vec::new()) };
}
