//! Compiled independently twice; never linked into the probe executable.
#[unsafe(no_mangle)]
pub fn probe_identity() -> usize {
    probe_shared::identity()
}
#[unsafe(no_mangle)]
pub fn probe_bump() -> usize {
    probe_shared::bump()
}
#[unsafe(no_mangle)]
pub fn probe_exchange(value: Vec<u8>) -> Vec<u8> {
    assert_eq!(probe_shared::exchange(value), [4, 5, 6]);
    // Allocate in this consumer, then drop the result in the executable.
    vec![4, 5, 6]
}
#[unsafe(no_mangle)]
pub fn probe_enter() -> mod_api::content::crash::ModGuard {
    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: "mod",
        hook: "cross-image",
        part: None,
    })
}
#[unsafe(no_mangle)]
pub fn probe_tls(expected: Option<&str>) {
    assert_eq!(mod_api::content::crash::current_native_mod(), expected);
}
