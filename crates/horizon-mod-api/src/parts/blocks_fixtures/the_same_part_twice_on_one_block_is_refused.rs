#[test]
pub(crate) fn the_same_part_twice_on_one_block_is_refused() {
    struct Other;
    #[mod_api::horizon_part]
    impl BlockPart for Other {}
    let mut r = super::PartRegistrar::new("first");
    let block_key = (&mut r as &mut dyn Registrar).block_key("test:gauge");
    (&mut r as &mut dyn Registrar).block_part(block_key, Gauge);
    r.mod_id = "second";
    r.mod_index = 1;
    let block_key = (&mut r as &mut dyn Registrar).block_key("test:gauge");
    (&mut r as &mut dyn Registrar).block_part(block_key, Other);
    let block_key = (&mut r as &mut dyn Registrar).block_key("test:gauge");
    (&mut r as &mut dyn Registrar).block_part(block_key, Gauge);
    (&mut r as &mut dyn Registrar).block_key("test:bad name");
    (&mut r as &mut dyn Registrar).block_part_tag("#test:gauged", Other);
    assert_eq!(r.parts.blocks["test:gauge"].len(), 2);
    assert_eq!(r.refused.len(), 3, "{:?}", r.refused);
    assert!(r.refused[0].contains("Gauge is attached to test:gauge twice, by first and second"));
    assert!(r.refused[1].contains("mod second: invalid registry key"));
    assert!(r.refused[2].contains("write it without #"));
}
