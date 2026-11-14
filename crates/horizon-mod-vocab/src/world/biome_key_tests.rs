use super::*;
use crate::wire::{Reader, Wire, Writer};
#[test]
fn biome_keys_refuse_previous_generations_and_unknown_ids() {
    let old = BiomeKeys::new(["minecraft:plains"]);
    let key = old.key("minecraft:plains").unwrap();
    assert_eq!(old.id(key), Some("minecraft:plains"));
    assert_eq!(old.key("test:nope"), None);
    let new = BiomeKeys::new(["minecraft:plains"]);
    assert_eq!(new.id(key), None);
    let mut bytes = Writer::new();
    key.put(&mut bytes);
    let bytes = bytes.into_bytes();
    drop(old);
    assert!(BiomeKey::take(&mut Reader::new(&bytes)).is_err());
}
