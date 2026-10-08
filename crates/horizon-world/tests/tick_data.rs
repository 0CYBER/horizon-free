use horizon_core::{
    position::BlockPos,
    ticks::{SavedTick, TickPriority, TickTypeId},
};
use horizon_world::{
    DATA_VERSION,
    chunk::ChunkData,
    nbt::{NamedTag, Tag},
    tick_data::{self, TickKind},
};

fn saved(pos: BlockPos) -> SavedTick {
    SavedTick {
        kind: TickTypeId(7),
        pos,
        delay: -1,
        priority: TickPriority::High,
    }
}
fn root() -> NamedTag {
    NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("xPos", Tag::Int(-1)),
            ("zPos", Tag::Int(-1)),
            ("sections", Tag::List(vec![])),
            ("unknown", Tag::Long(42)),
        ]),
    }
}
#[test]
fn numeric_codec_narrowing_and_malformed_data() {
    let tag = Tag::List(vec![Tag::compound([
        ("i", Tag::String("minecraft:stone".into())),
        ("x", Tag::Double((-1.5f64).to_bits())),
        ("y", Tag::Float(f32::NAN.to_bits())),
        ("z", Tag::Byte(-1)),
        ("t", Tag::Long(i64::MAX)),
        ("p", Tag::Double(f64::NEG_INFINITY.to_bits())),
    ])]);
    let ticks = tick_data::read_list(&tag, |_| Some(TickTypeId(7))).unwrap();
    assert_eq!(
        ticks,
        [SavedTick {
            pos: BlockPos::new(-1, 0, -1),
            priority: TickPriority::ExtremelyHigh,
            ..saved(BlockPos::default())
        }]
    );
    assert!(tick_data::read_list(&tag, |_| None).is_err());
    assert!(tick_data::read_list(&Tag::List(vec![Tag::Int(1)]), |_| Some(TickTypeId(0))).is_err());
    assert!(tick_data::read_list(&Tag::Int(0), |_| Some(TickTypeId(0))).is_err());
    assert!(tick_data::write_list(&ticks, |_| None).is_err());
}
#[test]
fn saved_chunk_ticks_filter_coordinates_and_write_atomically() {
    let mut root = root();
    let Tag::Compound(map) = &mut root.tag else {
        unreachable!()
    };
    map.insert(
        "block_ticks".into(),
        tick_data::write_list(
            &[
                saved(BlockPos::new(-1, 64, -1)),
                saved(BlockPos::new(0, 64, -1)),
            ],
            |_| Some("minecraft:stone".to_owned()),
        )
        .unwrap(),
    );
    let mut chunk = ChunkData::new(root).unwrap();
    let mut ticks = chunk
        .saved_ticks(TickKind::Block, |_| Some(TickTypeId(7)))
        .unwrap();
    assert_eq!(ticks.count(), 1);
    assert!(ticks.has_scheduled(BlockPos::new(-1, 64, -1), TickTypeId(7)));
    assert!(!ticks.has_scheduled(BlockPos::new(0, 64, -1), TickTypeId(7)));
    assert!(
        chunk
            .saved_ticks(TickKind::Fluid, |_| None)
            .unwrap()
            .peek()
            .is_none()
    );
    ticks.unpack(100);
    let packed = ticks.pack(100);
    let before = chunk.root().clone();
    assert!(
        chunk
            .set_saved_ticks(TickKind::Block, &packed, |_| None)
            .is_err()
    );
    assert_eq!(chunk.root(), &before);
    assert!(
        chunk
            .set_saved_ticks(TickKind::Block, &[saved(BlockPos::new(0, 64, -1))], |_| {
                Some("minecraft:stone".to_owned())
            })
            .is_err()
    );
    assert_eq!(chunk.root(), &before);
    chunk
        .set_saved_ticks(TickKind::Block, &packed, |_| {
            Some("minecraft:stone".to_owned())
        })
        .unwrap();
    assert_eq!(chunk.root().tag.get("unknown"), Some(&Tag::Long(42)));
    assert_eq!(
        chunk
            .saved_ticks(TickKind::Block, |_| Some(TickTypeId(7)))
            .unwrap()
            .count(),
        1
    );
}
