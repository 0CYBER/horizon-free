#[test]
fn an_answer_names_the_mod_that_gave_it() {
    struct Passer;
    #[mod_api::__subscriptions]
    impl Hooks for Passer {
        fn screen_opening(
            &self,
            _: &dyn ClientHost,
            _: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            None
        }
    }
    struct Opener;
    #[mod_api::__subscriptions]
    impl Hooks for Opener {
        fn screen_opening(
            &self,
            _: &dyn ClientHost,
            _: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            Some(ScreenOpening::ModScreen)
        }
    }
    let mods: Vec<ModEntry> = vec![
        ModEntry::new("passer", Box::new(Passer)),
        ModEntry::new("opener", Box::new(Opener)),
        ModEntry::new("late", Box::new(Opener)),
    ];
    install(mods.into());
    let view = ScreenView(&());
    let mut outcome = None;
    dispatch_screen_opening(&TestHost, &view, &view, &mut outcome);
    assert_eq!(
        outcome,
        Some(Answered {
            answer: ScreenOpening::ModScreen,
            mod_id: "opener"
        })
    );
}
