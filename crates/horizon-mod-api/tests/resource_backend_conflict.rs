//! A separate process owns this deliberately conflicting once-only installation.
use mod_api::wire::{ResourceAccess, WireError};

#[test]
fn competing_resource_backend_refuses_platform_initialization_permanently() {
    mod_api::wire::install_resource_access(ResourceAccess {
        intern: |_| Err(WireError::Invalid("foreign resource backend")),
        store: |_| Err(WireError::Invalid("foreign resource backend")),
        load: |_, _| Err(WireError::Invalid("foreign resource backend")),
        refused: |_| {},
    })
    .unwrap();
    for _ in 0..2 {
        assert_eq!(
            mod_api::__private::initialize(),
            Err("resource access is already installed")
        );
    }
}
