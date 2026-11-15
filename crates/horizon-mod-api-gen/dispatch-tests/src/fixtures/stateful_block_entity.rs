struct StateFactoryProbe(u32, Arc<Mutex<Vec<u32>>>);

impl BlockPart for StateFactoryProbe {
    fn block_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
}

impl BlockEntityPart for StateFactoryProbe {
    fn block_entity_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::StatefulBlockEntity), Some(HookId::BlockEntity)]
    }
    fn stateful_block_entity(&self, _: &dyn Host, event: &mut StatefulBlockEntityEvent<'_>) {
        assert_eq!(event.block, "fixture:heater");
        self.1.lock().unwrap().push(self.0);
        if self.0 == 2 {
            event.stateful = true;
        }
    }
    fn block_entity(&self, _: LevelSide<'_>, event: &mut BlockEntityEvent<'_>) -> Option<Compound> {
        assert_eq!(event.block, "fixture:heater");
        let _actual_state = event.state;
        self.1.lock().unwrap().push(self.0);
        (self.0 == 2).then_some(Compound)
    }
}

#[test]
fn state_factory_ownership_preserves_listen_order_and_existing_answers() {
    // These hooks belong to registered block entity Parts, not global Hooks.
    // No global subscribers may be needed for the Part dispatch to run.
    install(Vec::<ModEntry>::new().into());
    let calls = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        block_entities: (1..=3)
            .map(|id| leak(StateFactoryProbe(id, calls.clone())) as &dyn BlockEntityPart)
            .collect(),
        ..Default::default()
    });
    let mut stateful = false;
    dispatch_stateful_block_entity(&TestHost, "fixture:heater", &mut stateful);
    assert!(stateful);
    assert_eq!(*calls.lock().unwrap(), [1, 2, 3]);
    install_parts(Parts::default());
    dispatch_stateful_block_entity(&TestHost, "fixture:heater", &mut stateful);
    assert!(stateful, "no Parts must not clear ownership");
}

#[test]
fn actual_state_factory_preserves_none_and_stops_at_first_answer() {
    install(Vec::<ModEntry>::new().into());
    let calls = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        block_entities: vec![leak(StateFactoryProbe(1, calls.clone()))],
        ..Default::default()
    });
    let mut entity = None;
    dispatch_block_entity(
        LevelSide::Server(&TestLevel),
        "fixture:heater",
        BlockState::new(BlockKey::Test, 0),
        &mut entity,
    );
    assert!(entity.is_none());
    assert_eq!(*calls.lock().unwrap(), [1]);
    calls.lock().unwrap().clear();
    install_parts(Parts {
        block_entities: (1..=3)
            .map(|id| leak(StateFactoryProbe(id, calls.clone())) as &dyn BlockEntityPart)
            .collect(),
        ..Default::default()
    });
    dispatch_block_entity(
        LevelSide::Server(&TestLevel),
        "fixture:heater",
        BlockState::new(BlockKey::Test, 0),
        &mut entity,
    );
    assert!(entity.is_some());
    assert_eq!(
        *calls.lock().unwrap(),
        [1, 2],
        "later factories must not run"
    );
    install_parts(Parts::default());
}
