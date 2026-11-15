//! How a hook asks to move a level's clock, as data later listeners can read and change.
use crate::Identifier;

/// A move of a level's clock (NeoForge's `ClockAdjustment`). The game resolves it once every
/// listener has run. Case order is part of the wasm link contract.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ClockAdjustment {
    /// To this clock time, in ticks.
    Absolute(i64),
    /// This many ticks on from the clock's time now.
    Relative(i64),
    /// To the next time the level's clock reaches this time marker of its timelines, such as
    /// `minecraft:wake_up_from_sleep`.
    Marker(Identifier),
}

impl ClockAdjustment {
    /// The clock time this adjustment moves to from `current`, with `marker` answering when a
    /// marker is next reached; None for a marker the clock does not define and for a `Relative`
    /// time past the ends of `i64` (it never wraps).
    pub fn resolve(
        &self,
        current: i64,
        marker: impl FnOnce(&Identifier) -> Option<i64>,
    ) -> Option<i64> {
        match self {
            Self::Absolute(time) => Some(*time),
            Self::Relative(ticks) => current.checked_add(*ticks),
            Self::Marker(id) => marker(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ClockAdjustment;
    use crate::Identifier;

    #[test]
    fn each_kind_resolves_from_the_current_time() {
        let dawn = Identifier::parse("minecraft:wake_up_from_sleep").unwrap();
        let known = |id: &Identifier| (id == &dawn).then_some(24_000);
        assert_eq!(
            ClockAdjustment::Absolute(500).resolve(13_000, known),
            Some(500)
        );
        assert_eq!(
            ClockAdjustment::Relative(1_000).resolve(13_000, known),
            Some(14_000)
        );
        assert_eq!(
            ClockAdjustment::Marker(dawn.clone()).resolve(13_000, known),
            Some(24_000)
        );
        let other = Identifier::parse("example:mod_dawn").unwrap();
        assert_eq!(ClockAdjustment::Marker(other).resolve(13_000, known), None);
        assert_eq!(ClockAdjustment::Relative(i64::MAX).resolve(1, known), None);
        assert_eq!(ClockAdjustment::Relative(i64::MIN).resolve(-1, known), None);
        assert_eq!(
            ClockAdjustment::Relative(i64::MIN).resolve(0, known),
            Some(i64::MIN)
        );
        assert_eq!(
            ClockAdjustment::Absolute(i64::MAX).resolve(13_000, known),
            Some(i64::MAX)
        );
    }

    /// Every case crosses the wasm boundary and back, and a listener's replacement is what the
    /// host reads after it (the event's `&mut` field is written back through the same codec).
    #[test]
    fn every_case_survives_the_wire_and_writes_back() {
        use crate::wire::{Reader, Writer};
        let send = |value: &ClockAdjustment| {
            let mut out = Writer::new();
            out.put(value);
            Reader::decode::<ClockAdjustment>(out.bytes()).unwrap()
        };
        let cases = [
            ClockAdjustment::Absolute(i64::MIN),
            ClockAdjustment::Absolute(24_000),
            ClockAdjustment::Relative(-1_000),
            ClockAdjustment::Relative(i64::MAX),
            ClockAdjustment::Marker(Identifier::parse("minecraft:wake_up_from_sleep").unwrap()),
            ClockAdjustment::Marker(Identifier::parse("example:mod_dawn/late").unwrap()),
        ];
        for case in &cases {
            assert_eq!(&send(case), case);
        }
        // The guest gets the host's value, replaces it, and the host reads the replacement.
        let mut guest = send(&cases[2]);
        assert_eq!(guest, ClockAdjustment::Relative(-1_000));
        guest = ClockAdjustment::Marker(Identifier::parse("example:mod_dawn").unwrap());
        let host = send(&guest);
        assert_eq!(
            host,
            ClockAdjustment::Marker(Identifier::parse("example:mod_dawn").unwrap())
        );
    }
}
