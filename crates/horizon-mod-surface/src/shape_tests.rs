use super::*;

fn sha(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn record(fields: &[(&str, Shape)]) -> Definition {
    Definition {
        params: 0,
        body: Body::Record(
            fields
                .iter()
                .map(|(n, s)| (n.to_string(), s.clone()))
                .collect(),
        ),
    }
}

fn named(key: &str) -> Shape {
    Shape::Named(key.into(), vec![])
}

fn table(entries: Vec<(&str, Definition)>) -> BTreeMap<String, Definition> {
    entries
        .into_iter()
        .map(|(k, d)| (k.to_owned(), d))
        .collect()
}

#[test]
fn the_canonical_text_is_the_documented_one() {
    let defs = table(vec![
        (
            "Inner",
            Definition {
                params: 0,
                body: Body::Enum(vec![
                    ("A".into(), Body::Unit),
                    ("B".into(), Body::Tuple(vec![Shape::Scalar(Scalar::I64)])),
                ]),
            },
        ),
        (
            "Outer",
            record(&[
                ("id", Shape::Scalar(Scalar::U32)),
                (
                    "names",
                    Shape::List(Box::new(Shape::Option(Box::new(Shape::Scalar(
                        Scalar::Str,
                    ))))),
                ),
                ("inner", named("Inner")),
                (
                    "pair",
                    Shape::Array(Box::new(Shape::Opaque("glam::Vec3".into())), 2),
                ),
            ]),
        ),
    ]);
    let inner = sha("enum{1:Aunit{},1:Btuple{i64}}");
    let outer = sha(&format!(
        "record{{2:id:u32,5:names:list(option(str)),5:inner:type({inner}),4:pair:array(opaque(10:glam::Vec3);2)}}"
    ));
    assert_eq!(Shapes::new(&defs).shape("Outer").unwrap(), outer);
}

#[test]
fn reordering_fields_changes_the_shape() {
    let a = table(vec![(
        "T",
        record(&[
            ("x", Shape::Scalar(Scalar::U8)),
            ("y", Shape::Scalar(Scalar::U16)),
        ]),
    )]);
    let b = table(vec![(
        "T",
        record(&[
            ("y", Shape::Scalar(Scalar::U16)),
            ("x", Shape::Scalar(Scalar::U8)),
        ]),
    )]);
    assert_ne!(
        Shapes::new(&a).shape("T").unwrap(),
        Shapes::new(&b).shape("T").unwrap()
    );
}

#[test]
fn renaming_a_type_changes_no_shape_and_a_nested_change_propagates() {
    let base = table(vec![
        ("Leaf", record(&[("v", Shape::Scalar(Scalar::Bool))])),
        ("Other", record(&[("w", Shape::Scalar(Scalar::F32))])),
        ("Top", record(&[("leaf", named("Leaf"))])),
    ]);
    let renamed = table(vec![
        ("Leaf2", record(&[("v", Shape::Scalar(Scalar::Bool))])),
        ("Unrelated", record(&[("w", Shape::Scalar(Scalar::F32))])),
        ("Top", record(&[("leaf", named("Leaf2"))])),
    ]);
    let changed = table(vec![
        ("Leaf", record(&[("v", Shape::Scalar(Scalar::Char))])),
        ("Other", record(&[("w", Shape::Scalar(Scalar::F32))])),
        ("Top", record(&[("leaf", named("Leaf"))])),
    ]);
    let top = Shapes::new(&base).shape("Top").unwrap();
    assert_eq!(Shapes::new(&renamed).shape("Top").unwrap(), top);
    assert_ne!(Shapes::new(&changed).shape("Top").unwrap(), top);
}

#[test]
fn recursion_terminates_and_does_not_depend_on_order() {
    // A { b: Option<B> }, B { a: Vec<A> }, C { a: A }, and a self-recursive list node.
    let defs = table(vec![
        ("A", record(&[("b", Shape::Option(Box::new(named("B"))))])),
        ("B", record(&[("a", Shape::List(Box::new(named("A"))))])),
        ("C", record(&[("a", named("A"))])),
        (
            "Node",
            record(&[("next", Shape::Option(Box::new(named("Node"))))]),
        ),
    ]);
    let node = sha("record{4:next:option(rec(1))}");
    assert_eq!(Shapes::new(&defs).shape("Node").unwrap(), node);
    let b_in_a = sha("record{1:a:list(rec(2))}");
    let a = sha(&format!("record{{1:b:option(type({b_in_a}))}}"));
    let mut fresh = Shapes::new(&defs);
    assert_eq!(fresh.shape("A").unwrap(), a);
    let c = fresh.shape("C").unwrap();
    let b = fresh.shape("B").unwrap();
    let mut reversed = Shapes::new(&defs);
    assert_eq!(reversed.shape("B").unwrap(), b);
    assert_eq!(reversed.shape("C").unwrap(), c);
    assert_eq!(reversed.shape("A").unwrap(), a);
    assert_ne!(a, b);
}

#[test]
fn generic_arguments_are_part_of_the_shape() {
    let defs = table(vec![
        (
            "Wrap",
            Definition {
                params: 1,
                body: Body::Record(vec![("inner".into(), Shape::Param(0))]),
            },
        ),
        (
            "U",
            record(&[(
                "w",
                Shape::Named("Wrap".into(), vec![Shape::Scalar(Scalar::U32)]),
            )]),
        ),
        (
            "S",
            record(&[(
                "w",
                Shape::Named("Wrap".into(), vec![Shape::Scalar(Scalar::Str)]),
            )]),
        ),
    ]);
    let mut shapes = Shapes::new(&defs);
    assert_eq!(
        shapes.shape("Wrap").unwrap(),
        sha("record{5:inner:param(0)}")
    );
    assert_ne!(shapes.shape("U").unwrap(), shapes.shape("S").unwrap());
}

#[test]
fn a_bad_table_is_a_typed_error() {
    let defs = table(vec![
        ("Missing", record(&[("x", named("Nowhere"))])),
        (
            "Arity",
            record(&[("x", Shape::Named("Missing".into(), vec![Shape::Param(0)]))]),
        ),
        ("Param", record(&[("x", Shape::Param(3))])),
    ]);
    let mut shapes = Shapes::new(&defs);
    assert_eq!(
        shapes.shape("Missing"),
        Err(ShapeError::Undefined("Nowhere".into()))
    );
    assert_eq!(
        shapes.shape("Param"),
        Err(ShapeError::Param {
            key: "Param".into(),
            index: 3
        })
    );
    assert!(shapes.shape("Arity").is_err());
    assert_eq!(
        shapes.shape("Absent"),
        Err(ShapeError::Undefined("Absent".into()))
    );
}

#[test]
fn a_recursive_argument_counts_the_types_around_its_use() {
    // Wrap<T> { v: Option<T> }, A { f: Vec<Wrap<A>> } against W2 { v: Option<W2> }, C { f: Vec<W2> }:
    // after `Some`, an A continues with a list and a C with an option.
    let one = table(vec![
        (
            "Wrap",
            Definition {
                params: 1,
                body: Body::Record(vec![("v".into(), Shape::Option(Box::new(Shape::Param(0))))]),
            },
        ),
        (
            "A",
            record(&[(
                "f",
                Shape::List(Box::new(Shape::Named("Wrap".into(), vec![named("A")]))),
            )]),
        ),
    ]);
    let two = table(vec![
        ("W2", record(&[("v", Shape::Option(Box::new(named("W2"))))])),
        ("C", record(&[("f", Shape::List(Box::new(named("W2"))))])),
    ]);
    let a = Shapes::new(&one).shape("A").unwrap();
    let c = Shapes::new(&two).shape("C").unwrap();
    assert_ne!(a, c);
    // A's text: the argument A, used two levels below A, is rec(2).
    let wrap_of_a = sha("record{1:v:option(rec(2))}");
    assert_eq!(a, sha(&format!("record{{1:f:list(type({wrap_of_a}))}}")));
}

#[test]
fn nesting_past_the_limit_is_a_typed_error() {
    let mut entries = Vec::new();
    let names: Vec<String> = (0..=MAX_DEPTH).map(|i| format!("T{i}")).collect();
    for (i, name) in names.iter().enumerate() {
        let body = match names.get(i + 1) {
            Some(next) => record(&[("x", named(next))]),
            None => record(&[("x", Shape::Scalar(Scalar::U8))]),
        };
        entries.push((name.as_str(), body));
    }
    let defs = table(entries);
    assert!(matches!(
        Shapes::new(&defs).shape("T0"),
        Err(ShapeError::Depth(_))
    ));
    assert!(Shapes::new(&defs).shape("T1").is_ok());
}

#[test]
fn recursion_is_the_same_type_with_the_same_arguments() {
    // Pair<T> { a: T, b: Option<Pair<u16>> }, Holder { p: Pair<u8> }: Pair<u16> inside Pair<u8>
    // is another type, not a back reference.
    let defs = table(vec![
        (
            "Pair",
            Definition {
                params: 1,
                body: Body::Record(vec![
                    ("a".into(), Shape::Param(0)),
                    (
                        "b".into(),
                        Shape::Option(Box::new(Shape::Named(
                            "Pair".into(),
                            vec![Shape::Scalar(Scalar::U16)],
                        ))),
                    ),
                ]),
            },
        ),
        (
            "Holder",
            record(&[(
                "p",
                Shape::Named("Pair".into(), vec![Shape::Scalar(Scalar::U8)]),
            )]),
        ),
    ]);
    let pair_u16 = sha("record{1:a:u16,1:b:option(rec(1))}");
    let pair_u8 = sha(&format!("record{{1:a:u8,1:b:option(type({pair_u16}))}}"));
    let holder = sha(&format!("record{{1:p:type({pair_u8})}}"));
    assert_eq!(Shapes::new(&defs).shape("Holder").unwrap(), holder);
}
