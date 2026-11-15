//! An anvil level cost that does not fit in `ClientboundContainerSetDataPacket`.
//!
//! That packet's value is a short, which is what a vanilla client can store.
//! A cost outside `i16` changes sign there (`-65535` arrives as `+1`,
//! `i32::MIN` as `0`). A Horizon server still sends the short, then this
//! payload, and a Horizon client keeps the signed cost. Clientbound only.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};

/// The custom payload channel.
pub const CHANNEL: &str = "horizon:anvil_cost";

/// The cost `ContainerSetData` cannot carry unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cost {
    /// The anvil window the short was sent for.
    pub window: i32,
    pub cost: i32,
}

/// How `cost` travels: the vanilla short, and the full cost when that short
/// is not `cost`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Synced {
    pub short: i16,
    /// `Some` only when `short` cannot hold the cost.
    pub wide: Option<i32>,
}

/// The short a data slot sends, plus the wide cost when the short would not
/// be the same value.
pub fn sync(cost: i32) -> Synced {
    match i16::try_from(cost) {
        Ok(short) => Synced { short, wide: None },
        Err(_) => Synced {
            short: cost as i16,
            wide: Some(cost),
        },
    }
}

/// A message's payload.
pub fn encode(cost: Cost) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(cost.window);
    w.i32(cost.cost);
    w.data
}

/// A payload's cost; an error for one that is cut or has trailing bytes.
pub fn decode(data: &[u8]) -> Result<Cost> {
    let mut r = Reader::new(data);
    let window = r.varint()?;
    let cost = r.i32()?;
    if r.remaining() > 0 {
        return Err(Error::new("anvil cost: trailing bytes"));
    }
    Ok(Cost { window, cost })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cost_outside_the_short_keeps_its_sign() {
        // The short alone is the regression: -65535 becomes +1, i32::MIN becomes 0.
        assert_eq!(sync(-65535).short, 1);
        assert_eq!(sync(i32::MIN).short, 0);
        for cost in [
            i32::MIN,
            -65535,
            -32769,
            -32768,
            -7,
            0,
            32767,
            32768,
            i32::MAX,
        ] {
            let synced = sync(cost);
            let kept = synced.wide.unwrap_or(i32::from(synced.short));
            assert_eq!(kept, cost, "{cost}");
            assert_eq!(synced.wide.is_some(), i16::try_from(cost).is_err());
        }
    }

    #[test]
    fn a_wide_cost_round_trips() {
        for cost in [i32::MIN, -65535, -7, i32::MAX] {
            let payload = encode(Cost { window: 7, cost });
            assert_eq!(decode(&payload).unwrap(), Cost { window: 7, cost });
        }
        assert!(decode(&[]).is_err());
        let mut extra = encode(Cost {
            window: 1,
            cost: -1,
        });
        extra.push(0);
        assert!(decode(&extra).is_err(), "trailing bytes");
    }
}
