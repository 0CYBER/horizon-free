use mod_vocab::wire::{Reader, WireError};

#[test]
fn unregistered_fluid_storage_refuses_issuance_and_incoming_handles() {
    assert!(mod_vocab::__private::intern_unresolved_fluid("absent:fluid").is_none());
    assert_eq!(
        Reader::decode::<mod_vocab::UnresolvedFluid>(&0u64.to_le_bytes()),
        Err(WireError::Invalid(
            "unresolved fluid storage requires the platform runtime"
        ))
    );
}
