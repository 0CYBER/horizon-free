//! Assertions run inside the real wasm guest, not in a native stand-in.
pub fn read(host: &dyn mod_api::Host) {
    use mod_api::{DatapackReadError as E, DatapackValue, DimensionKey, LevelRef, WorldEpoch};
    let level = LevelRef::new(WorldEpoch::new(1), DimensionKey::OVERWORLD);
    assert_eq!(
        level.datapack_entries(host, "wasm_fixture:spell"),
        Some(vec!["addon:ice".into(), "wasm_fixture:fire".into()])
    );
    let handle = level
        .datapack_handle(host, "wasm_fixture:spell", "wasm_fixture:fire")
        .unwrap();
    assert_eq!(
        level.datapack_read(host, &handle),
        Ok(vec![("power".into(), DatapackValue::Int(7))])
    );
    assert_eq!(level.datapack_entries(host, "missing:registry"), None);
    assert_eq!(
        level.datapack_handle(host, "wasm_fixture:spell", "wasm_fixture:missing"),
        Err(E::MissingEntry)
    );
    let entry = host
        .registry_entry("wasm_fixture:spell", "wasm_fixture:fire")
        .unwrap();
    assert_eq!(
        entry.registry,
        mod_api::Identifier::parse("wasm_fixture:spell").unwrap()
    );
    assert_eq!(
        entry.id,
        mod_api::Identifier::parse("wasm_fixture:fire").unwrap()
    );
    assert_eq!(
        host.registry_entry_in_tag(&entry, "wasm_fixture:hot"),
        Some(true)
    );
    assert_eq!(
        host.registry_entry_in_tag(&entry, "wasm_fixture:absent"),
        Some(false)
    );
    assert_eq!(
        host.registry_entries("wasm_fixture:spell"),
        Some(vec![
            mod_api::Identifier::parse("addon:ice").unwrap(),
            mod_api::Identifier::parse("wasm_fixture:fire").unwrap(),
        ])
    );
    assert_eq!(
        host.registry_entry("wasm_fixture:spell", "wasm_fixture:missing"),
        None
    );
    assert_eq!(host.registry_entries("wasm_fixture:missing"), None);
    assert_eq!(entry.as_block_key(host), None);
    assert!(host.is_loaded("datapack_fixture_completed"));
}
