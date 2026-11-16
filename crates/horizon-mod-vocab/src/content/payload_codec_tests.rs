use super::*;

#[derive(Debug, PartialEq)]
struct NoCodec;

macro_rules! ident_ty {
    ($t:ty) => {
        $t
    };
}

#[derive(horizon_mod_api_macros::PayloadCodec)]
struct MacroField<T> {
    value: ident_ty!(T),
}

#[derive(Debug, PartialEq)]
struct Opaque<T>(u8, std::marker::PhantomData<T>);

impl<T> PayloadCodec for Opaque<T> {
    fn schema(_: &mut Vec<std::any::TypeId>) -> u64 {
        crate::content::payload_schema::payload_schema_leaf("test:Opaque")
    }
    fn encode(&self, out: &mut PayloadWriter) {
        self.0.encode(out);
    }

    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        u8::decode(input).map(|value| Self(value, std::marker::PhantomData))
    }
}

#[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
struct OpaqueEnvelope<T> {
    value: Opaque<T>,
}

/// The acceptance matrix's payload: `counter=7`.
#[derive(Debug, PartialEq)]
struct Counter {
    counter: i32,
    labels: Vec<String>,
}

impl PayloadCodec for Counter {
    fn schema(_: &mut Vec<std::any::TypeId>) -> u64 {
        crate::content::payload_schema::payload_schema_leaf("test:Counter")
    }
    fn encode(&self, out: &mut PayloadWriter) {
        out.var_int(self.counter);
        self.labels.encode(out);
    }
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        Ok(Self {
            counter: input.var_int()?,
            labels: Vec::decode(input)?,
        })
    }
}

/// A type that holds itself: `links` present options deep.
#[derive(Debug, PartialEq)]
struct Chain(Option<Box<Chain>>);

impl PayloadCodec for Chain {
    fn schema(_: &mut Vec<std::any::TypeId>) -> u64 {
        crate::content::payload_schema::payload_schema_leaf("test:Chain")
    }
    fn encode(&self, out: &mut PayloadWriter) {
        self.0.encode(out);
    }
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        Option::decode(input).map(Self)
    }
}

fn chain(links: usize) -> Chain {
    (0..links).fold(Chain(None), |inner, _| Chain(Some(Box::new(inner))))
}

/// `links` true tags and the false that ends them.
fn chain_bytes(links: usize) -> Vec<u8> {
    let mut bytes = vec![1; links];
    bytes.push(0);
    bytes
}

/// A zero-sized entry that counts each decode, to show none ran.
#[derive(Debug, PartialEq)]
struct Counted;

thread_local! {
    static DECODED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

impl PayloadCodec for Counted {
    fn schema(_: &mut Vec<std::any::TypeId>) -> u64 {
        crate::content::payload_schema::payload_schema_leaf("test:Counted")
    }
    fn encode(&self, _: &mut PayloadWriter) {}
    fn decode(_: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        DECODED.set(DECODED.get() + 1);
        Ok(Self)
    }
}

/// counter 7, then one label "a".
const SEVEN: [u8; 4] = [7, 1, 1, b'a'];

fn seven() -> Counter {
    Counter {
        counter: 7,
        labels: vec!["a".to_owned()],
    }
}

#[test]
fn a_value_has_vanillas_network_bytes_and_reads_back() {
    assert_eq!(encode_payload(&seven(), 64), Ok(SEVEN.to_vec()));
    assert_eq!(decode_payload::<Counter>(&SEVEN), Ok(seven()));

    let mut out = PayloadWriter::new(64);
    out.var_int(300);
    out.var_int(-1);
    out.var_long(-1);
    out.i32(-2);
    out.i16(0x0102);
    out.bool(true);
    out.f32(1.0);
    out.uuid(0x0102);
    out.string("é");
    let bytes = out.finish().unwrap();
    let mut expected = vec![0xAC, 0x02];
    expected.extend([0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    expected.extend([0xFF; 9]);
    expected.push(0x01);
    expected.extend([0xFF, 0xFF, 0xFF, 0xFE]);
    expected.extend([0x01, 0x02]);
    expected.push(1);
    expected.extend([0x3F, 0x80, 0, 0]);
    expected.extend([0; 14]);
    expected.extend([0x01, 0x02]);
    expected.extend([2, 0xC3, 0xA9]);
    assert_eq!(bytes, expected);

    let mut input = PayloadReader::new(&bytes);
    assert_eq!(input.var_int(), Ok(300));
    assert_eq!(input.var_int(), Ok(-1));
    assert_eq!(input.var_long(), Ok(-1));
    assert_eq!(input.i32(), Ok(-2));
    assert_eq!(input.i16(), Ok(0x0102));
    assert_eq!(input.bool(), Ok(true));
    assert_eq!(input.f32(), Ok(1.0));
    assert_eq!(input.uuid(), Ok(0x0102));
    assert_eq!(input.string().as_deref(), Ok("é"));
    assert_eq!(input.finish(), Ok(()));
}

/// The matrix row "Length -1, count 4097, depth 65, trailing byte": each is
/// refused, no entry is decoded, and the next body still reads as counter 7.
#[test]
fn hostile_bodies_are_refused_before_anything_is_reserved() {
    // A count of -1, then a count of 4097, each followed by nothing.
    let negative = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
    let over = [0x81, 0x20];
    DECODED.set(0);
    assert_eq!(
        decode_payload::<Vec<Counted>>(&negative),
        Err(PayloadDecodeError::NegativeLength { length: -1 })
    );
    assert_eq!(
        decode_payload::<Vec<Counted>>(&over),
        Err(PayloadDecodeError::LengthOverLimit {
            length: 4097,
            limit: 4096
        })
    );
    assert_eq!(DECODED.get(), 0);
    // The same two lengths on a string.
    assert_eq!(
        decode_payload::<String>(&negative),
        Err(PayloadDecodeError::NegativeLength { length: -1 })
    );
    assert_eq!(
        decode_payload::<String>(&over),
        Err(PayloadDecodeError::LengthOverLimit {
            length: 4097,
            limit: 4096
        })
    );

    assert_eq!(decode_payload(&chain_bytes(32)), Ok(chain(32)));
    assert_eq!(
        decode_payload::<Chain>(&chain_bytes(33)),
        Err(PayloadDecodeError::TooDeep)
    );

    let mut trailing = SEVEN.to_vec();
    trailing.push(0);
    assert_eq!(
        decode_payload::<Counter>(&trailing),
        Err(PayloadDecodeError::Trailing { bytes: 1 })
    );

    assert_eq!(decode_payload::<Counter>(&SEVEN), Ok(seven()));
}

#[test]
fn a_list_is_charged_its_decoded_size_before_it_is_reserved() {
    // 4096 absent options: 2 + 4096 bytes of body, 4096 * 16 bytes decoded.
    let mut body = vec![0x80, 0x20];
    body.extend([0; 4096]);
    let entry = std::mem::size_of::<Option<i64>>();
    let read = |budget| {
        let mut input = PayloadReader::with_budget(&body, budget);
        Vec::<Option<i64>>::decode(&mut input).map(|list| list.len())
    };
    assert_eq!(read(4096 * entry), Ok(4096));
    assert_eq!(
        read(4096 * entry - 1),
        Err(PayloadDecodeError::BudgetExhausted)
    );
    // Strings draw on the same budget: two of 3 bytes do not fit in 5.
    let strings = [2, 3, b'a', b'b', b'c', 3, b'd', b'e', b'f'];
    let mut input = PayloadReader::with_budget(&strings, 2 * std::mem::size_of::<String>() + 5);
    assert_eq!(
        Vec::<String>::decode(&mut input),
        Err(PayloadDecodeError::BudgetExhausted)
    );
}

#[test]
fn malformed_values_are_named() {
    use PayloadDecodeError::*;
    assert_eq!(decode_payload::<i32>(&[0, 0, 0]), Err(UnexpectedEnd));
    // A string of 3 bytes with 2 behind its length.
    assert_eq!(
        decode_payload::<String>(&[3, b'a', b'b']),
        Err(UnexpectedEnd)
    );
    assert_eq!(decode_payload::<String>(&[2, 0xC3, 0x28]), Err(Utf8));
    assert_eq!(
        decode_payload::<bool>(&[2]),
        Err(InvalidDiscriminant { value: 2 })
    );
    assert_eq!(
        decode_payload::<Option<u8>>(&[9, 1]),
        Err(InvalidDiscriminant { value: 9 })
    );
    assert_eq!(decode_payload::<VarInt>(&[0x80; 6]), Err(VarIntTooLong));
    assert_eq!(decode_payload::<VarLong>(&[0x80; 11]), Err(VarIntTooLong));
    assert_eq!(decode_payload::<VarInt>(&[0x80, 0x80]), Err(UnexpectedEnd));
    let nan = f64::NAN.to_be_bytes();
    assert_eq!(PayloadReader::new(&nan).finite_f64(), Err(NonFinite));
    assert!(PayloadReader::new(&nan).f64().is_ok_and(f64::is_nan));
    // A byte array bounded by its own schema, not by the list bound.
    let mut input = PayloadReader::new(&[3, 1, 2, 3]);
    assert_eq!(
        input.bytes(2),
        Err(LengthOverLimit {
            length: 3,
            limit: 2
        })
    );
    assert_eq!(
        PayloadReader::new(&[3, 1, 2, 3]).bytes(3),
        Ok(&[1, 2, 3][..])
    );
}

#[test]
fn an_encode_past_a_bound_writes_nothing() {
    use PayloadEncodeError::*;
    // 4 bytes fit in 4, not in 3.
    assert_eq!(encode_payload(&seven(), 4), Ok(SEVEN.to_vec()));
    assert_eq!(encode_payload(&seven(), 3), Err(TooLarge { limit: 3 }));
    assert_eq!(
        encode_payload(&"a".repeat(4097), usize::MAX),
        Err(LengthOverLimit {
            length: 4097,
            limit: 4096
        })
    );
    assert_eq!(
        encode_payload(&vec![0u8; 4097], usize::MAX),
        Err(LengthOverLimit {
            length: 4097,
            limit: 4096
        })
    );
    assert_eq!(encode_payload(&chain(32), usize::MAX), Ok(chain_bytes(32)));
    assert_eq!(encode_payload(&chain(33), usize::MAX), Err(TooDeep));
    let mut out = PayloadWriter::new(64);
    out.finite_f32(f32::INFINITY);
    // The first failure stays, and later writes add nothing.
    out.u8(1);
    assert_eq!(out.finish(), Err(NonFinite));
}

// Deliberately no Option or Vec: only Box can stop this hostile recursion.
#[derive(Debug, PartialEq)]
struct Recursive(Box<Recursive>);
impl PayloadCodec for Recursive {
    fn schema(_: &mut Vec<std::any::TypeId>) -> u64 {
        crate::content::payload_schema::payload_schema_leaf("test:Recursive")
    }
    fn encode(&self, out: &mut PayloadWriter) {
        self.0.encode(out);
    }
    fn decode(input: &mut PayloadReader<'_>) -> Result<Self, PayloadDecodeError> {
        input.u8()?;
        Box::decode(input).map(Self)
    }
}

#[test]
fn boxes_charge_allocation_and_bound_recursion() {
    assert_eq!(
        decode_payload::<Recursive>(&[0; 1024]),
        Err(PayloadDecodeError::TooDeep)
    );
    let body = [0, 0, 0, 7];
    let mut input = PayloadReader::with_budget(&body, 3);
    assert_eq!(
        Box::<i32>::decode(&mut input),
        Err(PayloadDecodeError::BudgetExhausted)
    );
    let mut input = PayloadReader::with_budget(&body, 4);
    assert_eq!(Box::<i32>::decode(&mut input), Ok(Box::new(7)));
}

#[test]
fn independent_float_varint_and_utf8_fixtures() {
    for bits in [0x80000000u32, 0x3fc00000, 0xffc12345] {
        let bytes = bits.to_be_bytes();
        assert_eq!(decode_payload::<f32>(&bytes).unwrap().to_bits(), bits);
        assert_eq!(encode_payload(&f32::from_bits(bits), 4), Ok(bytes.to_vec()));
    }
    for bits in [0x8000000000000000u64, 0xfff8123456789abc] {
        let bytes = bits.to_be_bytes();
        assert_eq!(decode_payload::<f64>(&bytes).unwrap().to_bits(), bits);
        assert_eq!(encode_payload(&f64::from_bits(bits), 8), Ok(bytes.to_vec()));
    }
    assert_eq!(PayloadReader::new(&[0x80, 0]).var_int(), Ok(0));
    assert_eq!(
        PayloadReader::new(&[0xff, 0xff, 0xff, 0xff, 0x7f]).var_int(),
        Ok(-1)
    );
    assert_eq!(PayloadReader::new(&[0x80, 0]).var_long(), Ok(0));
    assert_eq!(
        PayloadReader::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f])
            .var_long(),
        Ok(-1)
    );
    assert_eq!(
        PayloadReader::new(&[3, 0xe2, 0x82, 0xac]).str_within(3),
        Ok("€")
    );
    assert!(matches!(
        PayloadReader::new(&[3, 0xe2, 0x82, 0xac]).str_within(2),
        Err(PayloadDecodeError::LengthOverLimit { .. })
    ));
}

/// A derived record: its fields in declaration order, nothing between them.
#[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
struct Cell {
    out: VarInt,
    input: Option<String>,
    flags: u8,
}

#[test]
fn a_derived_codec_does_not_bound_generic_parameters_its_fields_do_not_need() {
    let envelope = OpaqueEnvelope {
        value: Opaque(42, std::marker::PhantomData::<NoCodec>),
    };
    assert_eq!(encode_payload(&envelope, 1), Ok(vec![42]));
    assert_eq!(
        decode_payload::<OpaqueEnvelope<NoCodec>>(&[42]),
        Ok(OpaqueEnvelope {
            value: Opaque(42, std::marker::PhantomData),
        })
    );
}

#[test]
fn a_derived_codec_bounds_a_type_parameter_hidden_in_a_type_macro() {
    let value = MacroField { value: 42_u8 };
    let bytes = [42];
    assert_eq!(encode_payload(&value, 1), Ok(bytes.to_vec()));
    assert_eq!(decode_payload::<MacroField<u8>>(&bytes).unwrap().value, 42);
}

#[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
struct GenericTree<T> {
    value: T,
    children: Vec<GenericTree<T>>,
}

#[test]
fn a_derived_generic_recursive_tree_uses_the_generic_bound() {
    let tree = GenericTree {
        value: 1_u8,
        children: vec![GenericTree {
            value: 2_u8,
            children: vec![],
        }],
    };
    let bytes = [1, 1, 2, 0];
    assert_eq!(encode_payload(&tree, 4), Ok(bytes.to_vec()));
    assert_eq!(
        decode_payload::<GenericTree<u8>>(&bytes),
        Ok(GenericTree {
            value: 1,
            children: vec![GenericTree {
                value: 2,
                children: vec![],
            }],
        })
    );
}

mod bound_overrides {
    use super::{NoCodec, Opaque, decode_payload, encode_payload};
    use crate::content::payload_codec as contract;

    #[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
    #[payload(crate = contract, bound = "T: contract::PayloadCodec")]
    struct A<T> {
        value: T,
        b: Vec<B<T>>,
    }

    #[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
    #[payload(crate = contract, bound = "T: contract::PayloadCodec")]
    struct B<T> {
        a: Option<Box<A<T>>>,
    }

    #[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
    #[payload(crate = contract, bound = "")]
    struct OpaqueTree<T> {
        value: Opaque<T>,
        children: Vec<OpaqueTree<T>>,
    }

    #[test]
    fn mutual_recursion_uses_explicit_bounds_and_independent_bytes() {
        let value = A {
            value: 1_u8,
            b: vec![B {
                a: Some(Box::new(A {
                    value: 2_u8,
                    b: vec![],
                })),
            }],
        };
        let bytes = [1, 1, 1, 2, 0];
        assert_eq!(encode_payload(&value, 5), Ok(bytes.to_vec()));
        assert_eq!(
            decode_payload::<A<u8>>(&bytes),
            Ok(A {
                value: 1,
                b: vec![B {
                    a: Some(Box::new(A {
                        value: 2,
                        b: vec![],
                    })),
                }],
            })
        );
    }

    #[test]
    fn empty_bound_override_supports_recursive_opaque_values() {
        let value = OpaqueTree {
            value: Opaque(7, std::marker::PhantomData::<NoCodec>),
            children: vec![OpaqueTree {
                value: Opaque(8, std::marker::PhantomData),
                children: vec![],
            }],
        };
        let bytes = [7, 1, 8, 0];
        assert_eq!(encode_payload(&value, 4), Ok(bytes.to_vec()));
        assert_eq!(
            decode_payload::<OpaqueTree<NoCodec>>(&bytes),
            Ok(OpaqueTree {
                value: Opaque(7, std::marker::PhantomData),
                children: vec![OpaqueTree {
                    value: Opaque(8, std::marker::PhantomData),
                    children: vec![],
                }],
            })
        );
    }
}

/// A derived enum: a VarInt tag counting variants from 0, then the variant's
/// fields.
#[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
enum Update<T> {
    Clear,
    Set(i16, T),
    Rename { name: String },
}

/// Discriminants do not change the tags: `Red` is 0 and `Blue` is 1.
#[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
enum Colour {
    Red = 7,
    Blue = 9,
}

#[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
struct Tree(Vec<Tree>);

#[test]
fn a_derived_codec_writes_fields_in_order_and_variants_by_position() {
    let cell = Cell {
        out: VarInt(300),
        input: Some("a".to_owned()),
        flags: 9,
    };
    let bytes = [0xAC, 0x02, 1, 1, b'a', 9];
    assert_eq!(encode_payload(&cell, 64), Ok(bytes.to_vec()));
    assert_eq!(decode_payload(&bytes), Ok(cell));

    for (update, bytes) in [
        (Update::Clear, &[0][..]),
        (Update::Set(0x0102, true), &[1, 1, 2, 1]),
        (
            Update::Rename {
                name: "é".to_owned(),
            },
            &[2, 2, 0xC3, 0xA9],
        ),
    ] {
        assert_eq!(encode_payload(&update, 64), Ok(bytes.to_vec()));
        assert_eq!(decode_payload(bytes), Ok(update));
    }
    assert_eq!(encode_payload(&Colour::Blue, 64), Ok(vec![1]));
    assert_eq!(decode_payload(&[0]), Ok(Colour::Red));
    assert_eq!(
        decode_payload::<Colour>(&[7]),
        Err(PayloadDecodeError::InvalidDiscriminant { value: 7 })
    );
}

#[test]
fn a_derived_codec_refuses_what_its_fields_refuse() {
    use PayloadDecodeError::*;
    // A tag past the last variant, and a tag of -1.
    assert_eq!(
        decode_payload::<Update<bool>>(&[3]),
        Err(InvalidDiscriminant { value: 3 })
    );
    assert_eq!(
        decode_payload::<Update<bool>>(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]),
        Err(InvalidDiscriminant { value: -1 })
    );
    // A field's own refusal comes through: the boolean of Set is 2.
    assert_eq!(
        decode_payload::<Update<bool>>(&[1, 0, 0, 2]),
        Err(InvalidDiscriminant { value: 2 })
    );
    assert_eq!(decode_payload::<Update<bool>>(&[1, 0]), Err(UnexpectedEnd));
    assert_eq!(
        decode_payload::<Update<bool>>(&[0, 0]),
        Err(Trailing { bytes: 1 })
    );
    // A derived type that holds itself is bounded by its lists: 64 lists, one
    // inside the other, fit and 65 do not.
    let nest = |lists: usize| {
        let mut bytes = vec![1; lists];
        bytes.push(0);
        bytes
    };
    assert!(decode_payload::<Tree>(&nest(63)).is_ok());
    assert_eq!(decode_payload::<Tree>(&nest(64)), Err(TooDeep));
    assert_eq!(
        encode_payload(&decode_payload::<Tree>(&nest(63)).unwrap(), 64),
        Ok(nest(63))
    );
}

#[cfg(test)]
mod derive_shapes {
    use super::{decode_payload, encode_payload};
    use crate::content::payload_codec as contract;

    // Deliberately shadow prelude types and result constructors.
    #[allow(dead_code)]
    struct Result;
    #[allow(dead_code)]
    struct Option;
    #[allow(dead_code)]
    struct Ok;
    #[allow(dead_code)]
    struct Err;

    #[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
    #[payload(crate = contract)]
    struct Tuple(u8);

    #[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
    struct Unit;

    #[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
    #[non_exhaustive]
    struct Generic<T, const N: usize>
    where
        T: Clone,
    {
        r#type: T,
    }

    #[derive(Debug, PartialEq, horizon_mod_api_macros::PayloadCodec)]
    enum Choice {
        One,
        Two(u8),
    }

    #[test]
    fn shapes_compile_and_match_independent_bytes() {
        assert_eq!(encode_payload(&Tuple(9), 1).unwrap(), [9]);
        assert_eq!(decode_payload::<Tuple>(&[9]).unwrap(), Tuple(9));
        assert_eq!(encode_payload(&Unit, 0).unwrap(), Vec::<u8>::new());
        assert_eq!(decode_payload::<Unit>(&[]).unwrap(), Unit);
        let value = Generic::<u8, 3> { r#type: 7 };
        assert_eq!(encode_payload(&value, 1).unwrap(), [7]);
        assert_eq!(decode_payload::<Generic<u8, 3>>(&[7]).unwrap(), value);
        assert_eq!(decode_payload::<Choice>(&[1, 5]).unwrap(), Choice::Two(5));
        assert!(decode_payload::<Choice>(&[2]).is_err());
    }
}
