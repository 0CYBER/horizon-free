//! Guest tests must use the same stack reservation as production guest workers.
pub(crate) fn on_guest_stack<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|scope| {
        horizon_guest_stack::builder()
            .spawn_scoped(scope, f)
            .expect("guest test thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}
