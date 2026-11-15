use super::*;
use crate::content::payload_codec::VarInt;
use horizon_mod_api_macros::PayloadCodec;

#[derive(PayloadCodec)]
struct Counter {
    counter: VarInt,
}

#[derive(PayloadCodec)]
struct Renamed {
    value: VarInt,
}

#[derive(PayloadCodec)]
struct Fixed {
    counter: i32,
}

#[derive(PayloadCodec)]
struct Pair(i32, String);

#[derive(PayloadCodec)]
struct Swapped(String, i32);

#[derive(PayloadCodec)]
enum Shape {
    Dot,
    Line(i32),
}

#[derive(PayloadCodec)]
enum ShapeRenamed {
    Point,
    Segment(i32),
}

#[derive(PayloadCodec)]
enum ShapeReordered {
    Line(i32),
    Dot,
}

#[derive(PayloadCodec)]
struct Tree {
    children: Vec<Tree>,
}

#[derive(PayloadCodec)]
#[payload(bound = "")]
struct Even {
    next: Option<Box<Odd>>,
}

#[derive(PayloadCodec)]
#[payload(bound = "")]
struct Odd {
    next: Option<Box<Even>>,
}

#[test]
fn the_schema_is_the_shape_of_the_bytes_not_the_rust_names() {
    assert_eq!(payload_schema::<Counter>(), payload_schema::<Renamed>());
    assert_eq!(payload_schema::<Shape>(), payload_schema::<ShapeRenamed>());
    // A VarInt and a fixed i32 write different bytes for the same number.
    assert_ne!(payload_schema::<Counter>(), payload_schema::<Fixed>());
    assert_ne!(payload_schema::<Pair>(), payload_schema::<Swapped>());
    assert_ne!(
        payload_schema::<Shape>(),
        payload_schema::<ShapeReordered>()
    );
    assert_ne!(
        payload_schema::<Vec<i32>>(),
        payload_schema::<Option<i32>>()
    );
}

#[test]
fn a_type_that_holds_itself_has_a_finite_schema() {
    let tree = payload_schema::<Tree>();
    assert_ne!(tree, payload_schema::<Vec<i32>>());
    // Even and Odd have the same shape, entered from either side.
    assert_eq!(payload_schema::<Even>(), payload_schema::<Odd>());
    assert_ne!(payload_schema::<Even>(), tree);
}

#[test]
fn the_fingerprint_is_pinned_for_every_build_and_target() {
    // Two peers compare this number at admission: a native and a wasm build of the same type, and
    // builds of different game versions, must agree. FNV-1a 64 of `struct { counter: VarInt }`:
    // "struct", then the field count 1 and the hash of "var_int" as little-endian u64s, computed
    // outside Rust (Python) from that definition.
    assert_eq!(payload_schema::<Counter>(), 0xf0f2_25e6_8e40_be70);
    assert_eq!(
        typed_payload_version("2", 0xab),
        "2+payload-schema.00000000000000ab"
    );
}
