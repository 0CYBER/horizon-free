use horizon_core::position::{BlockPos, ChunkPos};
use horizon_world::{block_entities::ProtoBlockEntities, nbt::Tag};

#[test]
fn invalid_pending_data_is_rejected_without_replacing_active_or_pending_entries() {
    let mut store = ProtoBlockEntities::new(ChunkPos::new(0, 0));
    let active = BlockPos::new(0, 64, 0);
    store.set_active(active, "active");
    let pending = Tag::compound([("x", Tag::Int(1)), ("y", Tag::Int(64)), ("z", Tag::Int(0))]);
    store.set_pending(pending.clone()).unwrap();
    assert!(store.set_pending(Tag::String("invalid".into())).is_err());
    assert_eq!(store.active(active), Some(&"active"));
    assert_eq!(store.pending(BlockPos::new(1, 64, 0)), Some(&pending));
}

#[test]
fn failed_active_serialization_never_falls_back_to_stale_pending_data() {
    let p = BlockPos::new(-1, 64, -1);
    let mut store = ProtoBlockEntities::new(ChunkPos::new(-1, -1));
    store
        .set_pending(Tag::compound([
            ("x", Tag::Int(15)),
            ("y", Tag::Int(64)),
            ("z", Tag::Int(15)),
        ]))
        .unwrap();
    store.set_active(p, 42);
    assert!(store.pending(p).is_none());
    let result = store.for_saving(p, |_| Err::<Tag, _>("serialization failed"));
    assert_eq!(result, Err("serialization failed"));
    assert_eq!(store.remove(p), Some(42));
    assert_eq!(
        store.for_saving(p, |_| Err::<Tag, _>("must not run")),
        Ok(None)
    );
}

#[test]
fn pending_save_snapshot_is_independent_and_mutating_coordinates_keeps_the_key() {
    let mut store = ProtoBlockEntities::<()>::new(ChunkPos::new(0, 0));
    let p = BlockPos::new(0, 0, 0);
    store
        .set_pending(Tag::compound([("payload", Tag::Long(42))]))
        .unwrap();
    let snapshot = store.for_saving(p, |_| Err::<Tag, _>(())).unwrap().unwrap();
    let Some(Tag::Compound(tag)) = store.pending_mut(p) else {
        panic!("pending")
    };
    tag.insert("x".into(), Tag::Int(123));
    assert_eq!(snapshot.get("x"), None);
    assert_eq!(store.pending(p).unwrap().get("x"), Some(&Tag::Int(123)));
    assert!(store.pending(BlockPos::new(123, 0, 0)).is_none());
}

#[test]
fn floating_coordinates_floor_before_saturating_to_java_int() {
    use horizon_world::block_entities::position_from_tag;
    for (value, expected) in [
        (Tag::Float(0xcf000001), i32::MIN),
        (Tag::Double(0xc1e0000000000001), i32::MIN),
        (Tag::Float(f32::NEG_INFINITY.to_bits()), i32::MIN),
        (Tag::Double(f64::INFINITY.to_bits()), i32::MAX),
        (Tag::Double(f64::NAN.to_bits()), 0),
        (Tag::Float((-1.5_f32).to_bits()), -2),
        (Tag::Double((-1.5_f64).to_bits()), -2),
    ] {
        let tag = Tag::compound([("y", value)]);
        assert_eq!(
            position_from_tag(ChunkPos::new(0, 0), &tag).unwrap().y,
            expected
        );
    }
}
