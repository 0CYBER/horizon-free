//! Thread-owned synchronous guest-dispatch failure fence shared by engine and loader.
thread_local! {
    static DISPATCH_FAILURE_GENERATION: std::rc::Rc<std::cell::Cell<u64>> =
        std::rc::Rc::new(std::cell::Cell::new(0));
}

/// Capture once when constructing the engine's thread-owned state. A changed
/// value means a guest call or response decode failed on this thread; preceding
/// host effects must not be interpreted as an accepted guest response.
#[doc(hidden)]
pub fn dispatch_failure_counter() -> std::rc::Rc<std::cell::Cell<u64>> {
    DISPATCH_FAILURE_GENERATION.with(std::rc::Rc::clone)
}

/// Mark the failed synchronous dispatch on the same thread as its host effects.
#[doc(hidden)]
pub fn record_dispatch_failure() {
    DISPATCH_FAILURE_GENERATION.with(|generation| {
        generation.set(generation.get().wrapping_add(1));
    });
}
