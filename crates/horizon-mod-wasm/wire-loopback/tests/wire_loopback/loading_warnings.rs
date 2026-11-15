use super::*;

#[test]
fn wire_loopback_guest_warning_reaches_the_host_under_its_mod() {
    let mods = [mod_api::ModEntry::new(
        "wire_test",
        Box::new(WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        }),
    )];
    mod_api::__private::install_parts(
        &mods,
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    )
    .unwrap();
    assert_eq!(
        mod_api::__private::registration_warnings("wire_test"),
        [mod_api::LoadingMessage::new(
            "wire_test.warn.optional",
            &["café", "jei"]
        )]
    );
}
