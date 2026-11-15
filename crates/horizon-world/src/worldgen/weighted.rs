//! Weighted choices with vanilla draw consumption and ordered entries.
use super::{DecodeError, Object, int_range, required};
use horizon_core::random::{InvalidBound, RandomSource};
use serde_json::value::RawValue;

#[derive(Clone, Debug)]
pub struct WeightedList<T> {
    entries: Vec<(T, i32)>,
    cumulative: Vec<i32>,
    total: i32,
}
impl<T> WeightedList<T> {
    /// Zero weights are retained. A positive total no greater than i32::MAX is
    /// required, and lists are bounded to 8192 entries.
    pub fn new(entries: Vec<(T, i32)>) -> Result<Self, DecodeError> {
        Self::build(entries, false)
    }
    fn build(entries: Vec<(T, i32)>, allow_empty: bool) -> Result<Self, DecodeError> {
        if entries.len() > 8192 {
            return Err(DecodeError::Field("weighted list size"));
        }
        let mut total = 0_i32;
        let mut cumulative = Vec::with_capacity(entries.len());
        for (_, weight) in &entries {
            if *weight < 0 {
                return Err(DecodeError::Field("negative weight"));
            }
            total = total.checked_add(*weight).ok_or(DecodeError::Field(
                "total weight exceeds vanilla maximum 2147483647",
            ))?;
            cumulative.push(total);
        }
        if total == 0 && !allow_empty {
            return Err(DecodeError::Field("empty weighted list"));
        }
        Ok(Self {
            entries,
            cumulative,
            total,
        })
    }
    pub fn entries(&self) -> &[(T, i32)] {
        &self.entries
    }
    pub fn sample(&self, random: &mut impl RandomSource) -> Result<&T, InvalidBound> {
        // Even a singleton with total weight one consumes a bounded draw.
        let index = random.next_int_bound(self.total)?;
        let entry = self.cumulative.partition_point(|end| *end <= index);
        Ok(&self.entries[entry].0)
    }
    pub(crate) fn decode(
        raw: &RawValue,
        decode: impl FnMut(&RawValue) -> Result<T, DecodeError>,
    ) -> Result<Self, DecodeError> {
        Self::decode_inner(raw, decode, false)
    }
    fn decode_inner(
        raw: &RawValue,
        mut decode: impl FnMut(&RawValue) -> Result<T, DecodeError>,
        allow_empty: bool,
    ) -> Result<Self, DecodeError> {
        let entries: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
        if entries.len() > 8192 {
            return Err(DecodeError::Field("weighted list size"));
        }
        Self::build(
            entries
                .iter()
                .map(|raw| {
                    let object: Object = serde_json::from_str(raw.get())?;
                    let data = decode(required(&object, "data")?)?;
                    let weight = int_range(required(&object, "weight")?, "weight", 0, i32::MAX)?;
                    Ok((data, weight))
                })
                .collect::<Result<_, DecodeError>>()?,
            allow_empty,
        )
    }
}

/// Optional choices preserve zero-weight entries without weakening the positive
/// total invariant required by integer and height providers.
#[derive(Clone, Debug)]
pub struct OptionalWeightedList<T>(WeightedList<T>);
impl<T> OptionalWeightedList<T> {
    pub fn new(entries: Vec<(T, i32)>) -> Result<Self, DecodeError> {
        WeightedList::build(entries, true).map(Self)
    }
    pub fn entries(&self) -> &[(T, i32)] {
        self.0.entries()
    }
    pub fn sample(&self, random: &mut impl RandomSource) -> Result<Option<&T>, InvalidBound> {
        if self.0.total == 0 {
            Ok(None)
        } else {
            self.0.sample(random).map(Some)
        }
    }
    pub(crate) fn decode(
        raw: &RawValue,
        decode: impl FnMut(&RawValue) -> Result<T, DecodeError>,
    ) -> Result<Self, DecodeError> {
        WeightedList::decode_inner(raw, decode, true).map(Self)
    }
}

/// Keep vanilla's bounded-int draw exactly; oversized extension pools consume
/// one nextLong and reduce its unsigned bits modulo the positive total.
pub(crate) fn draw(total: i64, random: &mut impl RandomSource) -> Option<i64> {
    if total <= 0 {
        None
    } else if total <= i64::from(i32::MAX) {
        random.next_int_bound(total as i32).ok().map(i64::from)
    } else {
        Some(((random.next_long() as u64) % total as u64) as i64)
    }
}

#[cfg(test)]
mod wide_weight_tests {
    use super::*;
    use horizon_core::random::LegacyRandom;

    #[test]
    fn wide_weights_draw_and_preserve_small_rng() {
        for seed in -20..20 {
            for total in [1, 2, 7, 100, i32::MAX] {
                let mut old = LegacyRandom::new(seed);
                let mut new = LegacyRandom::new(seed);
                assert_eq!(
                    draw(i64::from(total), &mut new),
                    Some(i64::from(old.next_int_bound(total).unwrap()))
                );
                assert_eq!(new.next_long(), old.next_long());
            }
            let total = 3 * i64::from(i32::MAX);
            let mut rng = LegacyRandom::new(seed);
            assert!((0..total).contains(&draw(total, &mut rng).unwrap()));
        }
    }

    #[test]
    fn wide_weights_decode_refuses_vanilla_overflow() {
        let raw = RawValue::from_string(format!("[{{\"data\":0,\"weight\":{0}}},{{\"data\":1,\"weight\":{0}}},{{\"data\":2,\"weight\":{0}}}]", i32::MAX)).unwrap();
        let error = WeightedList::decode(&raw, |_| Ok(())).unwrap_err();
        assert!(error.to_string().contains("vanilla maximum"));
    }
}
