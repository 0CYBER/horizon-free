use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The milliseconds a client tick lasts (`Minecraft.getTickTargetMillis`): `millis` comes
    /// in as the clock's, and a mod may set another.
    pub(in crate::game) fn native_tick_target_millis(&self, millis: &mut f32) {
        self.fire_tick_target_millis(millis);
    }
}

#[cfg(test)]
mod tick_target_millis_tests {
    use mod_api::{ClientHost, Hooks, ModEntry, TickTargetMillisEvent};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// A "timer": logs vanilla's tick length and answers `answer(millis)`.
    struct Timer(Arc<Mutex<Vec<f32>>>, fn(f32) -> f32);
    #[mod_api::__subscriptions]
    impl Hooks for Timer {
        fn tick_target_millis(&self, _: &dyn ClientHost, event: &mut TickTargetMillisEvent) {
            self.0.lock().unwrap().push(event.millis);
            event.millis = (self.1)(event.millis);
        }
    }

    fn with_timer(answer: fn(f32) -> f32, check: impl FnOnce(&mut super::Game<'_>)) -> Vec<f32> {
        let seen = Arc::default();
        let mods: mod_api::Mods = vec![ModEntry::new(
            "timer",
            Box::new(Timer(Arc::clone(&seen), answer)),
        )]
        .into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.options.execution_role = super::ExecutionRole::Client;
                check(game);
            });
        });
        seen.lock().unwrap().clone()
    }

    /// `Minecraft.getTickTargetMillis`: the mod gets the clock's tick length, the `/tick rate`
    /// included, and its answer paces the client; an answer that is not a positive number keeps
    /// vanilla's, and without a listener the length is vanilla's.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_sets_how_long_a_client_tick_lasts() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.execution_role = super::ExecutionRole::Client;
            assert_eq!(game.tick_target_millis(), 50.0);
        });
        let seen = with_timer(
            |millis| millis / 2.0,
            |game| {
                assert_eq!(game.tick_target_millis(), 25.0);
                game.clock.set_rate(10.0);
                assert_eq!(game.tick_target_millis(), 50.0);
            },
        );
        assert_eq!(seen, [50.0, 100.0]);
        let broken: [fn(f32) -> f32; 4] = [|_| f32::NAN, |_| 0.0, |_| -5.0, |_| f32::INFINITY];
        for broken in broken {
            let seen = with_timer(broken, |game| assert_eq!(game.tick_target_millis(), 50.0));
            assert_eq!(seen, [50.0]);
        }
    }
}
