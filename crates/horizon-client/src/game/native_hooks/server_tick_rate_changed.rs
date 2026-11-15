use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `MinecraftServer.onTickRateChanged`, before the autosave wait is adjusted: `/tick rate`
    /// set a rate or a `/tick sprint` ended.
    pub(in crate::game) fn native_server_tick_rate_changed(&mut self) {
        crate::game::level_control_hooks::LevelControl::TickRate
            .hear(|| self.fire_server_tick_rate_changed());
    }
}

#[cfg(test)]
mod server_tick_rate_changed_tests {
    use crate::game::native_guest_turn_tests::with_guests;
    use mod_api::{Hooks, ModEntry, ServerHost, ServerTickRateChangedEvent};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// A scheduling mod: logs the rate each notification reads, and runs `retune` on the first.
    struct Scheduler {
        rates: Arc<Mutex<Vec<f32>>>,
        retune: Option<&'static str>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Scheduler {
        fn server_tick_rate_changed(
            &self,
            host: &mut dyn ServerHost,
            _: &mut ServerTickRateChangedEvent,
        ) {
            let first = {
                let mut rates = self.rates.lock().unwrap();
                rates.push(host.tick_stats().0);
                rates.len() == 1
            };
            if let (true, Some(line)) = (first, self.retune) {
                assert!(host.commands().run_command(line));
            }
        }
    }

    fn with_scheduler(
        retune: Option<&'static str>,
        check: impl FnOnce(&mut super::Game<'_>, &dyn Fn() -> Vec<f32>),
    ) {
        let rates = Arc::new(Mutex::new(Vec::new()));
        let scheduler = Scheduler {
            rates: Arc::clone(&rates),
            retune,
        };
        let mods: mod_api::Mods = vec![ModEntry::new("scheduler", Box::new(scheduler))].into();
        with_mods(mods, || {
            with_guests(|game| check(game, &|| std::mem::take(&mut *rates.lock().unwrap())));
        });
    }

    /// `ServerTickRateManager.setTickRate` and `finishTickSprint`, vanilla's only two callers of
    /// `MinecraftServer.onTickRateChanged`: each route tells a mod once, the new rate readable,
    /// and leaves the autosave wait at 300 seconds of ticks when that is shorter.
    #[test]
    #[ignore = "daily"]
    fn a_mod_hears_every_route_of_on_tick_rate_changed_once() {
        with_scheduler(None, |game, heard| {
            assert_eq!(game.ticks_until_autosave, 6000);
            assert!(game.native_run_command("tick rate 10"));
            assert_eq!(heard(), [10.0]);
            assert_eq!(game.ticks_until_autosave, 3000);
            assert!(game.native_run_command("tick rate 40"));
            assert_eq!(heard(), [40.0]);
            assert_eq!(game.ticks_until_autosave, 3000);
            assert!(game.native_run_command("tick rate 40"));
            assert_eq!(heard(), [40.0]);

            assert!(!game.native_run_command("tick sprint stop"));
            assert!(game.native_run_command("tick sprint 5"));
            assert!(heard().is_empty());
            game.ticks_until_autosave = 100_000;
            assert!(game.native_run_command("tick sprint stop"));
            assert_eq!(heard(), [40.0]);
            assert_eq!(game.ticks_until_autosave, 12_000);

            assert!(game.native_run_command("tick sprint 5"));
            assert!(game.native_run_command("tick freeze"));
            assert_eq!(heard(), [40.0]);
            assert!(game.native_run_command("tick unfreeze"));
            assert!(heard().is_empty());

            assert!(game.native_run_command("tick rate 20"));
            assert_eq!(heard(), [20.0]);
            assert!(game.native_run_command("tick sprint 1"));
            game.ticks_until_autosave = 100_000;
            game.clock.world_runs();
            assert!(heard().is_empty());
            game.tick_server(false).unwrap();
            assert_eq!(heard(), [20.0]);
            // Completion shortens the wait before this tick decrements it.
            assert_eq!(game.ticks_until_autosave, 5999);
            game.tick_server(false).unwrap();
            assert!(heard().is_empty());
            assert_eq!(game.ticks_until_autosave, 5998);
        });
    }

    /// The hook comes before the wait is adjusted: a listener that sets another rate decides
    /// the wait, here 40 ticks a second keeping the 6000 that 10 would have halved.
    #[test]
    #[ignore = "daily"]
    fn a_listener_runs_before_the_autosave_wait_is_adjusted() {
        with_scheduler(Some("tick rate 40"), |game, heard| {
            assert!(game.native_run_command("tick rate 10"));
            assert_eq!(heard(), [10.0, 40.0]);
            assert_eq!(game.clock.rate(), 40.0);
            assert_eq!(game.ticks_until_autosave, 6000);
        });
    }
}
