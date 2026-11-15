//! Intentionally uninitialized dispatch belongs in its own process, not the lib tests.
use mod_api::__private::game::RegistrationContextNew;

#[test]
fn dispatch_before_installation_is_empty() {
    let mut components = Vec::new();
    mod_api::dispatch_component_types(
        &<mod_api::RegistrationContext as RegistrationContextNew>::new(),
        &mut components,
    );
    assert!(mod_api::__private::mods().is_empty());
    assert!(components.is_empty());
}
