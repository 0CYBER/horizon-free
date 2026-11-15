#[test]
pub(crate) fn registry_keys_resolve_reused_parts_and_rebind_without_changing_identity() {
    use mod_vocab::__private::parts_installation::block_id;
    struct Keys(Arc<std::sync::Mutex<Vec<BlockKey>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Keys {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let vanilla = r.block_key("stone");
            let same = r.block_key("minecraft:stone");
            let later = r.block_key("later:machine");
            assert_eq!(vanilla, same);
            assert_eq!(
                mod_vocab::__private::parts_installation::validate_block_key(
                    mod_vocab::__private::parts_installation::block_key_slot(vanilla)
                ),
                Ok(vanilla)
            );
            assert_eq!(
                mod_vocab::__private::parts_installation::validate_block_key(
                    mod_vocab::__private::parts_installation::block_key_slot(later)
                ),
                Ok(later)
            );
            assert!(
                mod_vocab::__private::parts_installation::validate_block_key(
                    mod_vocab::__private::parts_installation::block_key_slot(later) + 1
                )
                .is_err()
            );
            r.block_part(vanilla, Gauge);
            r.block_part(same, Connector);
            r.block_part(later, Gauge);
            *self.0.lock().unwrap() = vec![vanilla, same, later];
        }
    }
    let requested = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "requester",
        Box::new(Keys(requested.clone())),
    )]
    .into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
    let keys = requested.lock().unwrap().clone();
    assert_eq!(
        mod_vocab::__private::parts_installation::validate_block_key(
            mod_vocab::__private::parts_installation::block_key_slot(keys[0])
        ),
        Ok(keys[0])
    );
    assert!(mod_vocab::__private::parts_installation::validate_block_key(2).is_err());
    // The final registry contains content declared after the requester's registration.
    let states = ["minecraft:stone", "later:machine", "later:machine"];
    mod_vocab::__private::resolve_block_keys(["minecraft:stone", "later:machine"], |_| {
        let properties = std::collections::BTreeMap::new();
        horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
            .map_err(|error| format!("{error:?}"))
    })
    .unwrap();
    resolve_parts(
        &HashMap::new(),
        &HashMap::new(),
        KnownIds {
            block: &|id| states.contains(&id),
            item: &|_| false,
        },
    )
    .unwrap();
    let table = parts(HookId::AnalogOutput, "analog_output");
    let table = table.for_hook(HookId::AnalogOutput).unwrap();
    let value = crate::BlockState::from(crate::BlockStateRecord::new(keys[0], 0));
    assert_eq!(table.block_value(value).len(), 1);
    let invalid = crate::BlockState::from(crate::BlockStateRecord::new(keys[0], u16::MAX));
    assert!(table.block_value(invalid).is_empty());
    assert_eq!(block_id(keys[0]), Some("minecraft:stone"));
    assert_eq!(block_id(keys[2]), Some("later:machine"));
    assert!(mod_vocab::__private::parts_installation::validate_block_key(u32::MAX).is_err());
    assert_eq!(
        mod_vocab::__private::parts_installation::validate_block_key(
            mod_vocab::__private::parts_installation::block_key_slot(keys[0])
        ),
        Ok(keys[0])
    );
    assert_eq!(block_id(keys[0]), Some("minecraft:stone"));
}
