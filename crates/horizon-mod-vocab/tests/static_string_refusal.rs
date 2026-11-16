#[test]
fn unregistered_static_string_storage_refuses_clearly() {
    assert_eq!(
        mod_vocab::wire::intern("example:unregistered"),
        Err(mod_vocab::wire::WireError::Invalid(
            "static string interning requires the platform runtime"
        ))
    );
}
