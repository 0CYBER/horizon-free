//! Bytes a wasm mod and the game exchange that the core hash can't see: a flag's bits, the
//! network NBT an attachment carries and a mesh's JSON are written by code outside the codec
//! files the hash reads. A mod built before a change here would load and misread them, so each
//! encoding is pinned: changing one fails here, and needs `CORE_VERSION` bumped with it.
use horizon_core::nbt::{self, Compound, Limits, Tag};
use mod_vocab::wire::{Reader, Wire, Writer};
use mod_vocab::{BlockUpdateFlags, CubeListBuilder, MeshDefinition, PartPose};

fn bytes<T: Wire>(value: &T) -> Vec<u8> {
    let mut out = Writer::new();
    value.put(&mut out);
    out.into_bytes()
}

#[test]
fn block_update_flags_keep_their_bits() {
    let flags = [
        (BlockUpdateFlags::NONE, 0),
        (BlockUpdateFlags::NEIGHBORS, 1),
        (BlockUpdateFlags::CLIENTS, 2),
        (BlockUpdateFlags::INVISIBLE, 4),
        (BlockUpdateFlags::KNOWN_SHAPE, 16),
        (BlockUpdateFlags::SUPPRESS_DROPS, 32),
        (BlockUpdateFlags::MOVE_BY_PISTON, 64),
        (BlockUpdateFlags::SKIP_SHAPE_UPDATE_ON_WIRE, 128),
        (BlockUpdateFlags::SKIP_BLOCK_ENTITY_SIDE_EFFECTS, 256),
        (BlockUpdateFlags::SKIP_ON_PLACE, 512),
    ];
    for (flag, bits) in flags {
        assert_eq!(bytes(&flag), u32::to_le_bytes(bits), "{flag:?}");
        assert_eq!(
            Reader::decode::<BlockUpdateFlags>(&u32::to_le_bytes(bits)).unwrap(),
            flag
        );
    }
    let all = flags
        .iter()
        .fold(BlockUpdateFlags::NONE, |all, (flag, _)| all | *flag);
    assert_eq!(bytes(&all), [0xf7, 0x03, 0, 0]);
    // Bits the mod API doesn't name still cross, both ways.
    let unknown = Reader::decode::<BlockUpdateFlags>(&[0x08, 0, 0, 0x80]).unwrap();
    assert_eq!(bytes(&unknown), [0x08, 0, 0, 0x80]);
}

/// Every tag in one compound, as attachments cross (`AttachmentBatch`).
#[test]
fn network_nbt_keeps_its_bytes() {
    let entry = |name: &str, tag| (nbt::JavaString::from(name), tag);
    let inner: Compound = [entry("k", Tag::Byte(1))].into_iter().collect();
    let tag = Tag::Compound(
        [
            entry("a", Tag::Byte(-2)),
            entry("b", Tag::Short(0x1234)),
            entry("c", Tag::Int(0x0102_0304)),
            entry("d", Tag::Long(0x0102_0304_0506_0708)),
            entry("e", Tag::Float(1.0f32.to_bits())),
            entry("f", Tag::Double(1.0f64.to_bits())),
            entry("g", Tag::ByteArray(vec![1, 2])),
            entry("h", Tag::String("hé".into())),
            entry("i", Tag::List(vec![Tag::Int(5)])),
            entry("j", Tag::Compound(inner)),
            entry("k", Tag::IntArray(vec![7])),
            entry("l", Tag::LongArray(vec![9])),
            entry("m", Tag::List(Vec::new())),
        ]
        .into_iter()
        .collect(),
    );
    #[rustfmt::skip]
    let golden: &[u8] = &[
        0x0a,
        0x01, 0, 1, b'a', 0xfe,
        0x02, 0, 1, b'b', 0x12, 0x34,
        0x03, 0, 1, b'c', 1, 2, 3, 4,
        0x04, 0, 1, b'd', 1, 2, 3, 4, 5, 6, 7, 8,
        0x05, 0, 1, b'e', 0x3f, 0x80, 0, 0,
        0x06, 0, 1, b'f', 0x3f, 0xf0, 0, 0, 0, 0, 0, 0,
        0x07, 0, 1, b'g', 0, 0, 0, 2, 1, 2,
        0x08, 0, 1, b'h', 0, 3, b'h', 0xc3, 0xa9,
        0x09, 0, 1, b'i', 0x03, 0, 0, 0, 1, 0, 0, 0, 5,
        0x0a, 0, 1, b'j', 0x01, 0, 1, b'k', 1, 0,
        0x0b, 0, 1, b'k', 0, 0, 0, 1, 0, 0, 0, 7,
        0x0c, 0, 1, b'l', 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 9,
        0x09, 0, 1, b'm', 0x00, 0, 0, 0, 0,
        0x00,
    ];
    assert_eq!(nbt::write_network(&tag, Limits::default()).unwrap(), golden);
    assert_eq!(
        nbt::read_network(golden, Limits::default()).unwrap(),
        (tag, golden.len())
    );
}

/// A mesh crosses as JSON (`#[wire(via = String)]`).
#[test]
fn a_mesh_keeps_its_json() {
    let mut mesh = MeshDefinition::new();
    mesh.add_or_replace_child(
        mesh.root(),
        "head",
        CubeListBuilder::create().add_box([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0]),
        PartPose::offset(0.0, 24.0, 0.0),
    );
    let pose = |y: &str| {
        format!(
            r#"{{"x":0.0,"y":{y},"z":0.0,"x_rot":0.0,"y_rot":0.0,"z_rot":0.0,"x_scale":1.0,"y_scale":1.0,"z_scale":1.0}}"#
        )
    };
    let json = format!(
        r#"{{"parts":[{{"cubes":[],"pose":{},"children":[["head",1]]}},{{"cubes":[{{"origin":[-4.0,-8.0,-4.0],"dimensions":[8.0,8.0,8.0],"grow":{{"grow_x":0.0,"grow_y":0.0,"grow_z":0.0}},"mirror":false,"tex_coord":[0,0],"tex_scale":[1.0,1.0],"visible_faces":[true,true,true,true,true,true]}}],"pose":{},"children":[]}}]}}"#,
        pose("0.0"),
        pose("24.0")
    );
    assert_eq!(String::from(&mesh), json);
    assert_eq!(bytes(&mesh), bytes(&json));
    assert_eq!(
        Reader::decode::<MeshDefinition>(&bytes(&mesh)).unwrap(),
        mesh
    );
}
