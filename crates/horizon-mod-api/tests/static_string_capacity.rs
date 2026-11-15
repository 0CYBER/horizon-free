use mod_api::wire::{WireError, intern};

#[test]
fn full_static_string_storage_keeps_existing_identities_but_refuses_new_ones() {
    mod_api::__private::initialize().unwrap();
    let first = intern("capacity:id_0").unwrap();
    for index in 1..4096 {
        intern(&format!("capacity:id_{index}")).unwrap();
    }
    assert_eq!(
        intern("capacity:overflow"),
        Err(WireError::Invalid("too many static strings"))
    );
    assert!(std::ptr::eq(intern("capacity:id_0").unwrap(), first));
}
