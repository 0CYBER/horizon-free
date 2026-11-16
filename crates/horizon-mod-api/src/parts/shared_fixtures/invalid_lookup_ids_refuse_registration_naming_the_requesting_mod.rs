#[test]
pub(crate) fn invalid_lookup_ids_refuse_registration_naming_the_requesting_mod() {
    struct Invalid;
    #[mod_api::__subscriptions]
    impl Hooks for Invalid {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            for id in ["minecraft:", "Upper:chest", "test:has space"] {
                r.block_entity_type_key(id);
            }
        }
    }
    let errors = install_parts(
        &[ModEntry::new("lookup", Box::new(Invalid))],
        &crate::parts::tests::rule_schema(),
        |_| false,
    )
    .unwrap_err();
    assert_eq!(errors.len(), 3);
    assert!(
        errors
            .iter()
            .all(|error| error.starts_with("mod lookup: invalid registry key"))
    );
}
