#[test]
pub(crate) fn a_mods_tag_part_registered_first_runs_before_its_key_part() {
    let mut r = super::PartRegistrar::new("registrar-test");
    (&mut r as &mut dyn Registrar).block_part_tag("test:gauged", TagGauge);
    let block_key = (&mut r as &mut dyn Registrar).block_key("test:gauge");
    (&mut r as &mut dyn Registrar).block_part(block_key, Gauge);
    let tags = HashMap::from([(
        "test:gauged".to_owned(),
        HashSet::from(["test:gauge".to_owned()]),
    )]);
    let rows = mod_vocab::__private::parts_installation::merged_block_rows(
        r.parts,
        r.tags,
        &tags,
        "test:gauge",
    );
    let order: Vec<&str> = rows.iter().map(|a| a.type_name).collect();
    assert_eq!(order, [type_name::<TagGauge>(), type_name::<Gauge>()]);
}
