#[test]
pub(crate) fn a_tag_giving_a_part_twice_keeps_the_earlier_mod() {
    let part = |mod_index| Attached::<dyn BlockPart> {
        mod_id: ["earlier", "later"][mod_index],
        mod_index,
        order: 0,
        type_name: "Gauge",
        part: Arc::new(Gauge),
    };
    let table = HashMap::from([("test:gauge".into(), vec![part(1)])]);
    let tags = HashMap::from([(
        "test:gauged".to_owned(),
        HashSet::from(["test:gauge".to_owned()]),
    )]);
    let rows = mod_vocab::__private::parts_installation::merged_block_rows(
        Parts {
            blocks: table,
            ..Default::default()
        },
        TagParts {
            blocks: vec![("test:gauged".into(), part(0))],
            ..Default::default()
        },
        &tags,
        "test:gauge",
    );
    let kept: Vec<usize> = rows.iter().map(|a| a.mod_index).collect();
    assert_eq!(kept, [0]);
}
