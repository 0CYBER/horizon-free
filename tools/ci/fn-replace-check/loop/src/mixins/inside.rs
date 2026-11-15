fn inside(value: bool) -> bool {
    static SEEN: ::std::sync::atomic::AtomicU64 = ::std::sync::atomic::AtomicU64::new(0);
    let seen = SEEN.fetch_add(1, ::std::sync::atomic::Ordering::Relaxed);
    if seen % 2000 == 0 {
        fnr_loop::note(&format!("MODIFY_EXPRESSION_VALUE chunk check inside the loop, call {seen}"));
    }
    value
}
