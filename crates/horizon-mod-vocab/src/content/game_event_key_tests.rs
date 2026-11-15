use crate::{DataMap, DataMapError, GameEventKey, GameEventKeyError, Host, Tag};
struct Frequencies;
impl DataMap for Frequencies {
    type Entry = GameEventKey;
    type Value = i32;
    fn id(&self) -> &str {
        "neoforge:vibration_frequencies"
    }
    fn decode(&self, value: &Tag) -> Option<i32> {
        if let Tag::Int(value) = value {
            Some(*value)
        } else {
            None
        }
    }
}
struct Maps;
impl crate::host::keys::KeysBackend for Maps {
    fn game_event_key_id(&self, key: GameEventKey) -> Result<String, GameEventKeyError> {
        match key.0 {
            0 => Ok("minecraft:step".into()),
            1 => Ok("minecraft:swim".into()),
            _ => Err(GameEventKeyError::InvalidKey),
        }
    }
}
impl crate::host::data_maps::DataMapsBackend for Maps {
    fn data_map_value(
        &self,
        registry: &str,
        entry: &str,
        map: &str,
    ) -> Result<Option<Tag>, DataMapError> {
        assert_eq!(registry, "minecraft:game_event");
        assert_eq!(map, "neoforge:vibration_frequencies");
        Ok((entry == "minecraft:step").then_some(Tag::Int(7)))
    }
}
impl Host for Maps {
    fn backend(&self) -> Option<crate::BackendRef<'_>> {
        Some(crate::BackendRef::of(self))
    }
}
impl crate::Backend for Maps {}
crate::impl_host_areas!(Maps);
crate::impl_backend_areas!(Maps, except(keys, data_maps));
struct Bare;
impl Host for Bare {}
crate::impl_host_areas!(Bare);
#[test]
fn game_event_data_reads_exact_frequency_missing_and_invalid_slots() {
    assert_eq!(GameEventKey(0).id(&Maps), Ok("minecraft:step".into()));
    assert_eq!(GameEventKey(0).data(&Maps, &Frequencies), Ok(Some(7)));
    assert_eq!(GameEventKey(1).data(&Maps, &Frequencies), Ok(None));
    assert_eq!(
        GameEventKey(u32::MAX).id(&Maps),
        Err(GameEventKeyError::InvalidKey)
    );
    assert_eq!(
        GameEventKey(u32::MAX).data(&Maps, &Frequencies),
        Err(DataMapError::InvalidKey)
    );
    assert_eq!(
        GameEventKey(0).data(&Bare, &Frequencies),
        Err(DataMapError::Unavailable)
    );
    assert_eq!(
        GameEventKey(0).id(&Bare),
        Err(GameEventKeyError::Unavailable)
    );
    assert!(matches!(
        crate::wire::Reader::decode::<GameEventKey>(&u32::MAX.to_le_bytes()),
        Err(crate::wire::WireError::Invalid("unknown GameEventKey slot"))
    ));
}
