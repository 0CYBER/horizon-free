//! The world border a server's players see: all of it when they enter a
//! level (`PlayerList.sendLevelInfo`), then each change as
//! `BorderChangeListener` broadcasts it: a new centre, a new size, a size
//! change in progress, a new warning time or distance.
use crate::worldborder::WorldBorder;
use horizon_protocol::packets::play::{self, Clientbound as C};

/// `ClientboundInitializeBorderPacket`.
pub fn initialize(b: &WorldBorder) -> play::InitializeBorder {
    let [center_x, center_z] = b.center();
    play::InitializeBorder {
        center_x,
        center_z,
        old_size: b.size(),
        new_size: b.target_size(),
        lerp_time: b.remaining_ticks(),
        absolute_max_size: b.absolute_max_size(),
        warning_blocks: b.warning_distance,
        warning_time: b.warning_time,
    }
}

/// The border as the players were last told of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sent {
    center: [f64; 2],
    size: f64,
    target: f64,
    remaining: i64,
    warning_distance: i32,
    warning_time: i32,
}

impl Sent {
    pub fn of(b: &WorldBorder) -> Self {
        Self {
            center: b.center(),
            size: b.size(),
            target: b.target_size(),
            remaining: b.remaining_ticks(),
            warning_distance: b.warning_distance,
            warning_time: b.warning_time,
        }
    }

    /// The packets that tell the players what changed since `self`, other
    /// than a size change going on (which each client runs itself).
    pub fn changes(&self, b: &WorldBorder) -> Vec<C> {
        let now = Self::of(b);
        let mut out = Vec::new();
        if now.center != self.center {
            out.push(C::SetBorderCenter(play::SetBorderCenter {
                x: now.center[0],
                z: now.center[1],
            }));
        }
        // A change going on counts down a tick at a time (or not at all
        // while the game is frozen), towards the same size.
        let going_on = self.remaining > 0
            && now.target == self.target
            && (now.remaining == self.remaining || now.remaining == self.remaining - 1);
        if now.remaining > 0 {
            if !going_on {
                out.push(C::SetBorderLerpSize(play::SetBorderLerpSize {
                    old_size: now.size,
                    new_size: now.target,
                    lerp_time: now.remaining,
                }));
            }
        } else {
            // The size the players' border has: the one they were last
            // told of, or the target of a change that just ran its last
            // tick. A change ended early (a size set at once) still runs
            // on theirs, so any size is sent then.
            let theirs = match self.remaining {
                ..=0 => Some(self.size),
                1 => Some(self.target),
                _ => None,
            };
            if theirs != Some(now.size) {
                out.push(C::SetBorderSize(play::SetBorderSize { size: now.size }));
            }
        }
        if now.warning_time != self.warning_time {
            out.push(C::SetBorderWarningDelay(play::SetBorderWarningDelay {
                delay: now.warning_time,
            }));
        }
        if now.warning_distance != self.warning_distance {
            out.push(C::SetBorderWarningDistance(
                play::SetBorderWarningDistance {
                    blocks: now.warning_distance,
                },
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(packets: &[C]) -> Vec<&'static str> {
        packets
            .iter()
            .map(|p| match p {
                C::SetBorderCenter(_) => "center",
                C::SetBorderSize(_) => "size",
                C::SetBorderLerpSize(_) => "lerp",
                C::SetBorderWarningDelay(_) => "delay",
                C::SetBorderWarningDistance(_) => "distance",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn each_change_is_sent_once() {
        let mut b = WorldBorder::default();
        let mut sent = Sent::of(&b);
        assert!(sent.changes(&b).is_empty());
        b.set_center([4.0, -2.0]);
        b.set_size(10.0);
        b.warning_time = 30;
        assert_eq!(kinds(&sent.changes(&b)), ["center", "size", "delay"]);
        sent = Sent::of(&b);
        // A change over 3 ticks: sent once, then each client runs it.
        b.lerp_to(20.0, 3);
        assert_eq!(kinds(&sent.changes(&b)), ["lerp"]);
        for _ in 0..4 {
            sent = Sent::of(&b);
            b.tick();
            assert!(sent.changes(&b).is_empty(), "{:?}", sent.changes(&b));
        }
        assert_eq!(b.size(), 20.0);
        b.warning_distance = 1;
        assert_eq!(kinds(&Sent::of(&b).changes(&b)), [] as [&str; 0]);
        assert_eq!(kinds(&sent.changes(&b)), ["distance"]);
    }

    /// A size set at once while a change runs is sent, even the size the
    /// change was going to or the size the border stood at: each client
    /// would keep running the change for the rest of its ticks.
    #[test]
    fn a_change_ended_early_is_sent() {
        let moving = || {
            let mut b = WorldBorder::default();
            b.set_size(100.0);
            b.lerp_to(1000.0, 72_000);
            b.tick();
            b
        };
        // Set at once to where the change was going.
        let mut b = moving();
        let sent = Sent::of(&b);
        b.set_size(1000.0);
        let packets = sent.changes(&b);
        assert_eq!(kinds(&packets), ["size"]);
        assert_eq!(
            packets[0],
            C::SetBorderSize(play::SetBorderSize { size: 1000.0 })
        );
        // Stopped at the size it had when the players were last told of it.
        let mut b = moving();
        let sent = Sent::of(&b);
        let size = b.size();
        b.set_size(size);
        let packets = sent.changes(&b);
        assert_eq!(kinds(&packets), ["size"]);
        assert_eq!(packets[0], C::SetBorderSize(play::SetBorderSize { size }));
    }
}
