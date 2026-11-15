/// Fail before capability-dependent fixtures silently choose the legacy launch path.
pub(crate) fn assert_holder_capability() {
    horizon_mod_package::holder::retain_capability();
    assert!(
        horizon_mod_package::holder::capable(&std::env::current_exe().unwrap()).unwrap(),
        "capability marker missing from this binary (release optimisation?)"
    );
}

#[test]
fn current_binary_retains_holder_capability() {
    assert_holder_capability();
}
