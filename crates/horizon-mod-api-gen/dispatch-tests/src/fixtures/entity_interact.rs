#[test]
fn entity_interact_reports_each_typed_hand() {
    struct Listener;
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn entity_interact(
            &self,
            _: HostSide<'_>,
            event: &mut EntityInteractEvent<'_>,
        ) -> Option<InteractionResult> {
            assert!(matches!(event.hand, Hand::Main | Hand::Off));
            if event.hand == Hand::Off {
                Some(InteractionResult::SuccessServer)
            } else {
                None
            }
        }
    }
    install(vec![ModEntry::new("hands", Box::new(Listener))].into());
    for hand in [Hand::Main, Hand::Off] {
        let mut result = None;
        dispatch_entity_interact(
            HostSide::Server(&mut TestHost),
            hand,
            42,
            "minecraft:zombie",
            None,
            &mut result,
        );
        assert_eq!(
            result,
            (hand == Hand::Off).then_some(InteractionResult::SuccessServer)
        );
    }
}
