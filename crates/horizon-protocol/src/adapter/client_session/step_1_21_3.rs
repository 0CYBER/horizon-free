//! 1.21.4's packets in 1.21.3's layout for a 1.21.2 or 1.21.3 client (protocol 768), after 1.21.4's step:
//! - level_particles: 1.21.4 added `always_show` after `override_limiter`; it goes.
//! - player_info_update: 1.21.4 added UPDATE_HAT (bit 7, a bool last in each entry); it goes, UPDATE_LIST_ORDER
//!   (bit 6, 1.21.2's) stays.
//!
//! (Checked against the 26.3 jar's ClientboundPlayerInfoUpdatePacket.Action order and both releases' layouts.)
use super::step_1_21_1;
use crate::{Result, State, ids as packet_ids};

pub(super) fn clientbound(state: State, id: i32, payload: &[u8]) -> Result<Option<Vec<u8>>> {
    use packet_ids::play::clientbound as cb;
    if state != State::Play {
        return Ok(None);
    }
    Ok(match id {
        cb::LEVEL_PARTICLES => Some(step_1_21_1::particles(payload)?),
        cb::PLAYER_INFO_UPDATE => Some(step_1_21_1::player_info(payload, true)?),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buf::{Reader, Writer};

    #[test]
    fn the_hat_goes_and_the_list_order_stays() {
        let mut w = Writer::new();
        w.u8(0x40 | 0x80 | 0x08); // listed, list order, hat
        w.varint(1);
        w.uuid(42);
        w.bool(true); // listed
        w.varint(7); // list order
        w.bool(true); // hat
        let out = clientbound(
            State::Play,
            packet_ids::play::clientbound::PLAYER_INFO_UPDATE,
            &w.data,
        )
        .unwrap()
        .unwrap();
        let mut r = Reader::new(&out);
        assert_eq!(r.u8().unwrap(), 0x48);
        assert_eq!(r.varint().unwrap(), 1);
        assert_eq!(r.uuid().unwrap(), 42);
        assert!(r.bool().unwrap());
        assert_eq!(r.varint().unwrap(), 7);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn particles_lose_always_show() {
        let out = clientbound(
            State::Play,
            packet_ids::play::clientbound::LEVEL_PARTICLES,
            &[1, 0, 9, 9],
        )
        .unwrap()
        .unwrap();
        assert_eq!(out, vec![1, 9, 9]);
    }
}
