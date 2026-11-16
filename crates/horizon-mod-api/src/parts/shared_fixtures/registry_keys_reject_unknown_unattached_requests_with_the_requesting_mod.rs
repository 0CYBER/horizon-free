#[test]
pub(crate) fn registry_keys_reject_unknown_unattached_requests_with_the_requesting_mod() {
    struct Unknown;
    #[mod_api::__subscriptions]
    impl Hooks for Unknown {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.block_key("absent:block");
            r.effect_key("absent:effect");
        }
    }
    install_parts(
        &[ModEntry::new("lookup", Box::new(Unknown))],
        &crate::parts::tests::rule_schema(),
        |_| false,
    )
    .unwrap();
    assert_eq!(
        mod_vocab::__private::resolve_block_keys(std::iter::empty(), |_| unreachable!())
            .unwrap_err(),
        ["mod lookup requested unknown block absent:block"]
    );
    assert_eq!(
        mod_vocab::__private::resolve_effects(std::iter::empty(), |_| None).unwrap_err(),
        ["mod lookup requested unknown effect absent:effect"]
    );
}
