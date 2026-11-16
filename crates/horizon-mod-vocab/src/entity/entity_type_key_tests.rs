use crate::{
    EntityTypeKey,
    content::registry_keys,
    wire::{Reader, Writer},
};

#[test]
fn entity_type_key_derive_checks_host_slots_and_round_trips() {
    registry_keys::resolve_entity_types(["minecraft:zombie", "minecraft:player"]).unwrap();
    let zombie = registry_keys::resolved_entity_type_key("minecraft:zombie").unwrap();
    let player = registry_keys::resolved_entity_type_key("minecraft:player").unwrap();
    assert_ne!(zombie, player);
    assert_eq!(
        registry_keys::resolved_entity_type_key("zombie"),
        Some(zombie)
    );
    assert_eq!(
        registry_keys::resolved_entity_type_key_parts(&["minecraft:", "zombie"]),
        Some(zombie)
    );
    assert_eq!(
        registry_keys::resolved_entity_type_key("test:missing"),
        None
    );
    let mut bytes = Writer::new();
    bytes.put(&zombie);
    assert_eq!(bytes.bytes(), &u32::from(&zombie).to_le_bytes());
    assert_eq!(
        Reader::decode::<EntityTypeKey>(bytes.bytes()).unwrap(),
        zombie
    );
    assert!(Reader::decode::<EntityTypeKey>(&u32::MAX.to_le_bytes()).is_err());
    assert_eq!(
        registry_keys::entity_type_key_id(zombie).as_deref(),
        Some("minecraft:zombie")
    );
}
