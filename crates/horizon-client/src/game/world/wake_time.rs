//! Where sleeping through the night moves the level's clock: vanilla's morning, or the mods'
//! `ClockAdjustment` from `sleep_finished_time` (NeoForge's `SleepFinishedTimeEvent`), and the
//! clock time markers both read.
use super::{Dim, Game};
use crate::game::chat::{OVERWORLD_CLOCK, TimeMarker};
use mod_api::{ClockAdjustment, Identifier, LevelRef};

/// Vanilla's wake-up time marker, where sleeping through the night moves the clock
/// (`ClockTimeMarkers.WAKE_UP_FROM_SLEEP`).
const WAKE_UP_FROM_SLEEP: &str = "minecraft:wake_up_from_sleep";
/// A day on the clock, in ticks.
const DAY: i64 = 24_000;

/// Why the game refused a mod's adjustment; the clock then moves to vanilla's morning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refused {
    /// The time is before the clock's time now (`setTimeAddition` refuses a lower one).
    BeforeNow,
    /// The level's clock defines no such marker.
    UndefinedMarker,
    /// The time does not fit the clock (past `i64::MAX`).
    OutOfRange,
}

impl Game<'_> {
    /// The time marker `id` of `clock`. The timelines' markers belong to the Overworld's clock:
    /// no other clock defines one today, as `/time set <marker>` reads them.
    fn clock_marker(&self, clock: &str, id: &Identifier) -> Option<TimeMarker> {
        if clock != OVERWORLD_CLOCK {
            return None;
        }
        let id = format!("{}:{}", id.namespace(), id.path());
        self.ui.markers.get(&id).copied()
    }

    /// The host read `level_clock_marker_time`: when the level's own clock next reaches `marker`
    /// after its time now, as `/time set <marker>` moves it. None for a marker that clock does not
    /// define (every clock but the Overworld's today), a time past `i64::MAX`, and as for
    /// `level_clock_time`.
    pub(in crate::game) fn level_clock_marker_time(
        &self,
        level: LevelRef,
        marker: &Identifier,
    ) -> Option<i64> {
        let now = self.level_clock_time(level)?;
        let clock = self.level_clock_name(Dim::from_id(level.dimension_id()));
        self.clock_marker(&clock, marker)?.resolve(now)
    }

    /// The level's clock, its time now and where sleeping moves it, after the mods; None when a
    /// mod canceled the move, or when no time fits (the clock is at the end of its range).
    pub(super) fn sleep_finished_move(&mut self) -> Option<(String, i64, i64)> {
        let clock = self.level_clock_name(self.dim);
        let current = self.command_clock_time(&clock);
        let morning = vanilla_morning(current);
        let vanilla =
            ClockAdjustment::Marker(Identifier::parse(WAKE_UP_FROM_SLEEP).expect("a valid id"));
        let mut adjustment = vanilla.clone();
        let mut cancel = false;
        let dimension = self.dim.id();
        // `new_time` is the morning, or the time now when no morning fits.
        let shown = morning.unwrap_or(current);
        self.native_sleep_finished_time(dimension, current, shown, &mut adjustment, &mut cancel);
        if cancel {
            return None;
        }
        let target = match wake_time(current, &adjustment, |id| self.clock_marker(&clock, id)) {
            Ok(time) => Some(time),
            Err(why) => {
                // Vanilla's own marker on a clock without it is the plain morning, no news.
                if adjustment != vanilla {
                    log::warn!("sleep_finished_time: {adjustment:?} refused ({why:?})");
                }
                morning
            }
        };
        target.map(|target| (clock, current, target))
    }
}

/// The morning after `current`, as the game woke sleepers before the markers: the next whole day.
fn vanilla_morning(current: i64) -> Option<i64> {
    current.checked_add(DAY - current.rem_euclid(DAY))
}

/// Where `adjustment` moves the clock from `current`, `marker` giving the level's clock's markers.
fn wake_time(
    current: i64,
    adjustment: &ClockAdjustment,
    marker: impl FnOnce(&Identifier) -> Option<TimeMarker>,
) -> Result<i64, Refused> {
    let time = match adjustment {
        ClockAdjustment::Marker(id) => marker(id).ok_or(Refused::UndefinedMarker)?.resolve(current),
        other => other.resolve(current, |_| None),
    }
    .ok_or(Refused::OutOfRange)?;
    if time < current {
        return Err(Refused::BeforeNow);
    }
    Ok(time)
}

#[cfg(test)]
mod tests {
    use super::{Refused, WAKE_UP_FROM_SLEEP, vanilla_morning, wake_time};
    use crate::game::chat::TimeMarker;
    use mod_api::{ClockAdjustment, Identifier};

    const NOW: i64 = 13_000;

    fn id(s: &str) -> Identifier {
        Identifier::parse(s).unwrap()
    }
    fn marker(s: &str) -> ClockAdjustment {
        ClockAdjustment::Marker(id(s))
    }
    /// The Overworld clock's markers as vanilla ships the two used here.
    fn overworld(id: &Identifier) -> Option<TimeMarker> {
        let at = |ticks| TimeMarker {
            ticks,
            period: Some(24_000),
            show: false,
        };
        match format!("{}:{}", id.namespace(), id.path()).as_str() {
            WAKE_UP_FROM_SLEEP => Some(at(0)),
            "minecraft:noon" => Some(at(6_000)),
            _ => None,
        }
    }
    /// A clock that defines no markers, as every clock but the Overworld's today.
    fn bare(_: &Identifier) -> Option<TimeMarker> {
        None
    }

    #[test]
    fn vanillas_marker_is_the_morning_as_before() {
        for now in [0, 1, NOW, 23_999, 24_000, 48_001] {
            assert_eq!(
                wake_time(now, &marker(WAKE_UP_FROM_SLEEP), overworld).ok(),
                vanilla_morning(now),
                "{now}"
            );
        }
        assert_eq!(vanilla_morning(NOW), Some(24_000));
        assert_eq!(vanilla_morning(24_000), Some(48_000));
    }

    #[test]
    fn a_mods_adjustment_moves_the_clock_where_it_says() {
        let at = |a: ClockAdjustment| wake_time(NOW, &a, overworld);
        assert_eq!(at(ClockAdjustment::Relative(1_000)), Ok(14_000));
        assert_eq!(at(ClockAdjustment::Absolute(20_000)), Ok(20_000));
        assert_eq!(at(ClockAdjustment::Absolute(NOW)), Ok(NOW));
        assert_eq!(at(marker("minecraft:noon")), Ok(30_000));
    }

    #[test]
    fn a_time_before_now_or_an_undefined_marker_is_refused() {
        let at = |a: ClockAdjustment| wake_time(NOW, &a, overworld);
        assert_eq!(
            at(ClockAdjustment::Absolute(NOW - 1)),
            Err(Refused::BeforeNow)
        );
        assert_eq!(at(ClockAdjustment::Relative(-5)), Err(Refused::BeforeNow));
        assert_eq!(
            at(ClockAdjustment::Relative(i64::MIN)),
            Err(Refused::BeforeNow)
        );
        assert_eq!(
            at(marker("example:mod_dawn")),
            Err(Refused::UndefinedMarker)
        );
    }

    /// The wake marker has no special case: a clock without it refuses it like any marker, and
    /// the sleep then falls back to the plain morning.
    #[test]
    fn a_clock_without_markers_refuses_even_the_wake_marker() {
        assert_eq!(
            wake_time(NOW, &marker(WAKE_UP_FROM_SLEEP), bare),
            Err(Refused::UndefinedMarker)
        );
        assert_eq!(
            wake_time(NOW, &ClockAdjustment::Relative(1_000), bare),
            Ok(14_000)
        );
    }

    /// Mod data at the ends of the range never overflows: a time past `i64::MAX` is refused, and
    /// a clock a mod left at `i64::MAX` has no morning to move to.
    #[test]
    fn times_at_the_end_of_the_range_are_refused_not_wrapped() {
        assert_eq!(
            wake_time(NOW, &ClockAdjustment::Absolute(i64::MAX), overworld),
            Ok(i64::MAX)
        );
        assert_eq!(
            wake_time(NOW, &ClockAdjustment::Relative(i64::MAX), overworld),
            Err(Refused::OutOfRange)
        );
        let end = i64::MAX;
        assert_eq!(vanilla_morning(end), None);
        assert_eq!(vanilla_morning(end - 5), None);
        assert_eq!(
            wake_time(end, &marker(WAKE_UP_FROM_SLEEP), overworld),
            Err(Refused::OutOfRange)
        );
        assert_eq!(
            wake_time(end - 5, &marker("minecraft:noon"), overworld),
            Err(Refused::OutOfRange)
        );
        assert_eq!(
            wake_time(end, &ClockAdjustment::Relative(0), overworld),
            Ok(end)
        );
        // The last morning that fits.
        let last = end - end.rem_euclid(24_000);
        assert_eq!(vanilla_morning(last - 1), Some(last));
    }

    /// What a second listener reads: the first moves the clock 1000 ticks on, the second sees
    /// that and sends it to noon instead, and the game resolves the last word once.
    #[test]
    #[ignore = "daily"]
    fn a_later_listener_reads_and_replaces_the_earlier_adjustment() {
        use crate::game::chat::OVERWORLD_CLOCK;
        use std::sync::{Arc, Mutex};
        struct First;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for First {
            fn sleep_finished_time(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::SleepFinishedTimeEvent<'_>,
            ) -> mod_api::Flow {
                *event.adjustment = ClockAdjustment::Relative(1_000);
                mod_api::Flow::Continue
            }
        }
        struct Second(Arc<Mutex<Option<ClockAdjustment>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Second {
            fn sleep_finished_time(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::SleepFinishedTimeEvent<'_>,
            ) -> mod_api::Flow {
                *self.0.lock().unwrap() = Some(event.adjustment.clone());
                *event.adjustment = marker("minecraft:noon");
                mod_api::Flow::Continue
            }
        }
        crate::game::native_entity_refs::tests::with_game(|game| {
            let noon = TimeMarker {
                ticks: 6_000,
                period: Some(24_000),
                show: false,
            };
            game.ui.markers.insert("minecraft:noon".into(), noon);
            let clock = game.level_clock_name(game.dim);
            assert_eq!(clock, OVERWORLD_CLOCK);
            game.set_command_clock_time(&clock, NOW);
            let seen = Arc::new(Mutex::new(None));
            let mods = vec![
                mod_api::ModEntry::new("first", Box::new(First) as Box<dyn mod_api::Hooks>),
                mod_api::ModEntry::new(
                    "second",
                    Box::new(Second(seen.clone())) as Box<dyn mod_api::Hooks>,
                ),
            ]
            .into();
            let moved = mod_platform::testing::with_mods(mods, || game.sleep_finished_move());
            assert_eq!(
                *seen.lock().unwrap(),
                Some(ClockAdjustment::Relative(1_000))
            );
            assert_eq!(moved, Some((OVERWORLD_CLOCK.to_owned(), NOW, 30_000)));
        });
    }
}
