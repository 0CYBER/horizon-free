/// A restitution part: records the side its level host is on and the factor it was given, then
/// answers its own.
struct RestitutionProbe(f32, Arc<Mutex<Vec<(bool, f32)>>>);

impl BlockPart for RestitutionProbe {
    fn block_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::BounceRestitution)]
    }
    fn bounce_restitution(&self, level: LevelSide<'_>, event: &mut BounceRestitutionEvent<'_>) {
        assert_eq!((event.entity, event.kind), (7, "fixture:pig"));
        let client = matches!(level, LevelSide::Client(_));
        self.1.lock().unwrap().push((client, event.factor));
        event.factor = self.0;
    }
}

#[test]
fn restitution_parts_read_the_level_of_either_side_and_answer_in_turn() {
    // A block part with no global subscriber: the parts of the state answer.
    install(Vec::<ModEntry>::new().into());
    let heard = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        blocks: vec![(
            BlockKey::Test,
            [0.5, 0.25]
                .map(|answer| leak(RestitutionProbe(answer, heard.clone())) as &dyn BlockPart)
                .into(),
        )],
        ..Default::default()
    });
    for (level, client) in [
        (LevelSide::Server(&TestLevel), false),
        (LevelSide::Client(&TestLevel), true),
    ] {
        heard.lock().unwrap().clear();
        let mut factor = 1.0;
        dispatch_bounce_restitution(
            level,
            7,
            "fixture:pig",
            BlockState::new(BlockKey::Test, 0),
            BlockPos,
            &mut factor,
        );
        // The second part is given the first part's answer, and its own is the last word.
        assert_eq!(factor, 0.25);
        assert_eq!(*heard.lock().unwrap(), [(client, 1.0), (client, 0.5)]);
    }
    install_parts(Parts::default());
}
