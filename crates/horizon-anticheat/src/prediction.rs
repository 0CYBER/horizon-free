//! The movement prediction's record: each of a player's client ticks as the server heard them, so
//! the server can play the tick again with vanilla's movement code and compare (docs/anticheat.md).
//!
//! A vanilla client ends every tick with ServerboundClientTickEndPacket, after the tick's input
//! (ServerboundPlayerInputPacket, sent when the keys change) and its move (ServerboundMovePlayerPacket,
//! when it moved or turned). Those packets arrive in order, so the packets between two tick ends are one
//! client tick.
//!
//! What the server changes about the player (a push, a teleport, an effect) takes effect on the client
//! when it handles the packet, which is some client ticks after the server sent it. At the end of each of
//! its ticks the server sends a ClientboundPingPacket whose id is that server tick; the client answers
//! with ServerboundPongPacket as soon as it handles it, before its next tick runs. A client tick knows
//! the last server tick it had handled: every change of that server tick or an earlier one was in effect
//! during the client tick, none of a later one.

use std::collections::VecDeque;

/// Client ticks kept: two seconds at 20 a second.
pub const KEPT: usize = 40;

/// One client tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientTick {
    /// The keys held (ServerboundPlayerInputPacket's flags: forward, backward, left, right, jump,
    /// shift, sprint), as last sent.
    pub input: u8,
    /// Sprinting, as the player's commands left it (ServerboundPlayerCommandPacket).
    pub sprinting: bool,
    /// The rotation the client last sent.
    pub yaw: f32,
    pub pitch: f32,
    /// Where the tick's move said the player is, if it said: a move with a position.
    pub position: Option<[f64; 3]>,
    /// The tick's move packet's flags (1: on the ground, 2: against a wall), if it sent one.
    pub flags: Option<u8>,
    /// The last server tick whose packets the client had handled (its pong) before this tick ended.
    pub seen: Option<u64>,
}

/// A player's last client ticks.
#[derive(Clone, Debug, Default)]
pub struct Record {
    /// The tick being heard.
    current: ClientTick,
    /// The ended ones, oldest first, at most [`KEPT`].
    ticks: VecDeque<ClientTick>,
    /// The last server tick the client answered for.
    seen: Option<u64>,
}

impl Record {
    /// The tick's move: its position if it has one, the rotation if it has one, and its flags.
    pub fn moved(&mut self, position: Option<[f64; 3]>, rotation: Option<(f32, f32)>, flags: u8) {
        if position.is_some() {
            self.current.position = position;
        }
        if let Some((yaw, pitch)) = rotation {
            (self.current.yaw, self.current.pitch) = (yaw, pitch);
        }
        self.current.flags = Some(flags);
    }

    /// ServerboundPongPacket for the ping sent at the end of `server_tick`. False for an answer to a
    /// ping that was never sent, or one older than an answer already heard: a vanilla client answers
    /// every ping, once, in order.
    pub fn pong(&mut self, server_tick: u64, sent_up_to: u64) -> bool {
        if server_tick > sent_up_to || self.seen.is_some_and(|seen| server_tick <= seen) {
            return false;
        }
        self.seen = Some(server_tick);
        true
    }

    /// ServerboundClientTickEndPacket: the tick ends with these keys held and this sprinting. The
    /// next tick starts with the same keys and rotation, until the client says otherwise.
    pub fn tick_end(&mut self, input: u8, sprinting: bool) -> &ClientTick {
        self.current.input = input;
        self.current.sprinting = sprinting;
        self.current.seen = self.seen;
        let next = ClientTick {
            yaw: self.current.yaw,
            pitch: self.current.pitch,
            ..ClientTick::default()
        };
        let ended = std::mem::replace(&mut self.current, next);
        if self.ticks.len() == KEPT {
            self.ticks.pop_front();
        }
        self.ticks.push_back(ended);
        self.ticks.back().expect("just pushed")
    }

    /// The ended ticks, oldest first.
    pub fn ticks(&self) -> impl DoubleEndedIterator<Item = &ClientTick> + ExactSizeIterator {
        self.ticks.iter()
    }

    /// The last server tick the client answered for.
    pub fn seen(&self) -> Option<u64> {
        self.seen
    }

    /// A teleport or a respawn: the ticks before it say nothing about the ones after.
    pub fn clear(&mut self) {
        self.ticks.clear();
        self.current = ClientTick {
            yaw: self.current.yaw,
            pitch: self.current.pitch,
            ..ClientTick::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tick_is_the_packets_between_two_tick_ends() {
        let mut r = Record::default();
        r.moved(Some([1.0, 2.0, 3.0]), Some((90.0, 10.0)), 1);
        let t = *r.tick_end(0b1001, true);
        assert_eq!(t.position, Some([1.0, 2.0, 3.0]));
        assert_eq!((t.yaw, t.pitch, t.flags), (90.0, 10.0, Some(1)));
        assert_eq!((t.input, t.sprinting, t.seen), (0b1001, true, None));
        // A tick without a move keeps the rotation and has no position.
        let t = *r.tick_end(0, false);
        assert_eq!((t.position, t.flags, t.yaw), (None, None, 90.0));
        assert_eq!(r.ticks().len(), 2);
    }

    #[test]
    fn pongs_say_which_server_ticks_a_client_tick_had_seen() {
        let mut r = Record::default();
        assert!(r.pong(5, 6));
        r.tick_end(0, false);
        assert!(r.pong(6, 6));
        r.tick_end(0, false);
        r.tick_end(0, false);
        let seen: Vec<_> = r.ticks().map(|t| t.seen).collect();
        assert_eq!(seen, [Some(5), Some(6), Some(6)]);
        // Never sent, answered twice, or out of order.
        assert!(!r.pong(9, 6));
        assert!(!r.pong(6, 6));
        assert!(!r.pong(4, 6));
        assert_eq!(r.seen(), Some(6));
    }

    #[test]
    fn only_the_last_two_seconds_are_kept() {
        let mut r = Record::default();
        for i in 0..KEPT + 5 {
            r.moved(Some([i as f64, 0.0, 0.0]), None, 0);
            r.tick_end(0, false);
        }
        assert_eq!(r.ticks().len(), KEPT);
        assert_eq!(r.ticks().next().unwrap().position, Some([5.0, 0.0, 0.0]));
        r.clear();
        assert_eq!(r.ticks().len(), 0);
    }
}
