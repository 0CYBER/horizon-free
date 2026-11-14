use super::*;
use crate::wire::{Reader, Wire, Writer};
#[test]
fn biome_keys_refuse_previous_generations_and_unknown_ids() {
    let old = BiomeKeys::new(1, ["minecraft:plains"]);
    let key = old.key("minecraft:plains").unwrap();
    assert_eq!(old.id(key), Ok("minecraft:plains"));
    assert_eq!(old.key("test:nope"), None);
    let new = BiomeKeys::new(2, ["minecraft:plains"]);
    assert_eq!(new.id(key), Err(BiomeKeyError::WrongGeneration));
    let mut bytes = Writer::new();
    key.put(&mut bytes);
    let bytes = bytes.into_bytes();
    drop(old);
    let decoded = BiomeKey::take(&mut Reader::new(&bytes)).unwrap();
    assert_eq!(new.id(decoded), Err(BiomeKeyError::WrongGeneration));
}
