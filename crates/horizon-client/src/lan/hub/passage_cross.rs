//! The hub's half of a guest's crossing of a passage within its level (#3775,
//! docs/modding/src/how-to/passages.md): the move to where the client crossed from, checked
//! as any move; the position packet that accepts the crossing; and a refusal. The game finds
//! the passage, asks the mods and works out where the player comes out, in between.
use super::*;
use horizon_protocol::packets::configuration::CustomPayload;
use horizon_protocol::passage_sync::{self, Message};
use mod_api::PassageCrossRefusal as Refusal;

/// How far from the nearest point of the opening a crossing may start, in blocks: Immersive
/// Portals' `getDistanceToNearestPointInPortal` limit.
pub const REACH: u32 = 20;

/// The server's refusals as they travel: a refusal's code is its place here.
const REFUSALS: [Refusal; 12] = [
    Refusal::UnknownPassage,
    Refusal::Pending,
    Refusal::NotLoaded,
    Refusal::Dead,
    Refusal::Sleeping,
    Refusal::Riding,
    Refusal::MoveRefused,
    Refusal::TooFar { limit: REACH },
    Refusal::NearEndNotTracked,
    Refusal::FarEndNotTracked,
    Refusal::Vetoed,
    Refusal::FarLevel,
];

/// Where a crossing starts on the server: the client's `from` clamped as a move's position
/// is (`ServerGamePacketListenerImpl.clampHorizontal`, `clampVertical`).
pub fn crossing_source(from: [f64; 3]) -> [f64; 3] {
    [
        from[0].clamp(-3.0E7, 3.0E7),
        from[1].clamp(-2.0E7, 2.0E7),
        from[2].clamp(-3.0E7, 3.0E7),
    ]
}

/// The refusal's code on `horizon:passage_sync`.
pub fn refusal_code(refusal: Refusal) -> u8 {
    REFUSALS.iter().position(|r| *r == refusal).unwrap_or(0) as u8
}

/// The refusal a code names; none for a code this build does not know.
pub fn refusal_named(code: u8) -> Option<Refusal> {
    REFUSALS.get(usize::from(code)).copied()
}

impl Hub {
    /// A guest's `horizon:passage_cross` payload, in its place among the guest's packets:
    /// the move to where it crossed from is checked as a move packet would be (vanilla's
    /// numbers, from the server's last good position, on the ground the server has it on),
    /// though it is no client tick's move. Passed, the crossing goes to the game and true is
    /// returned: the rest of the guest's packets wait for the next tick. Refused, the
    /// client is told why and put back where the server has the player. Malformed or not
    /// a number: kicked, as a move with invalid values is.
    pub(super) fn passage_leg(
        &mut self,
        i: usize,
        payload: CustomPayload,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        let conn = self.guests[i].conn;
        let cross = match passage_sync::decode_cross(&payload.data) {
            Ok(cross) if !cross.from.iter().any(|v| v.is_nan()) => cross,
            _ => {
                self.kick(conn, invalid_movement());
                return false;
            }
        };
        if let Some(why) = self.passage_refusal(i) {
            self.refuse_passage(i, cross.id, why);
            return false;
        }
        // No client tick: the prediction's record starts again at the confirmation.
        let g = &self.guests[i];
        let flags = u8::from(g.on_ground);
        let rot = (g.yaw, g.pitch);
        // The caller reads the position at once, so the report is committed in place, never held.
        self.position_change(i, cross.from, rot, flags, Hearing::Crossing, ctx, events);
        let g = &self.guests[i];
        if g.awaiting_teleport.is_some() || g.position != crossing_source(cross.from) {
            // The move check sent its own correction, or kept the player where they were.
            if g.awaiting_teleport.is_none() {
                self.resend_teleport(i, None);
            }
            self.tell_refusal(i, cross.id, Refusal::MoveRefused);
            return false;
        }
        events.push(HostEvent::Packet(conn, Box::new(S::CustomPayload(payload))));
        // Nothing of the guest's overtakes the game's decision, a second crossing least of
        // all: it would move the player before the first is decided from its own `from`.
        true
    }

    /// Why guest `i` may not cross a passage now, whatever the passage: the checks before
    /// the move to where it crossed from, which the game makes again after the mods' veto.
    pub fn passage_refusal(&self, i: usize) -> Option<Refusal> {
        let g = &self.guests[i];
        if g.dead {
            Some(Refusal::Dead)
        } else if g.landing || !g.client_loaded() {
            Some(Refusal::NotLoaded)
        } else if g.awaiting_teleport.is_some() {
            Some(Refusal::Pending)
        } else if g.vehicle.is_some() {
            Some(Refusal::Riding)
        } else if g.sleeping {
            Some(Refusal::Sleeping)
        } else {
            None
        }
    }

    /// The game accepted guest `i`'s crossing: the server moves the player to `to`, turns
    /// its look by `turn` degrees and its motion with `motion`, starts its move checks again
    /// from there (`ServerGamePacketListenerImpl.resetPosition`, as `ServerPlayer.teleport`
    /// and Immersive Portals do), and sends a position packet that is all relative and zero:
    /// the client, already there, stays put and keeps its velocity. Moves until its
    /// confirmation wait, as for any teleport, and the confirmation is checked from `to`.
    /// The fall distance and the flying ticks go on.
    pub fn follow_passage(
        &mut self,
        i: usize,
        to: [f64; 3],
        turn: f32,
        motion: impl FnOnce([f64; 3]) -> [f64; 3],
    ) {
        self.guests.update(i, |g| {
            g.yaw = wrap_degrees(g.yaw + turn);
            g.head_yaw = g.yaw;
            g.first_good = to;
            if let Some(m) = g.motion.as_mut() {
                let [x, y, z] = motion([m.velocity.x, m.velocity.y, m.velocity.z]);
                m.velocity = horizon_core::geometry::Vec3::new(x, y, z);
            }
        });
        use play::relative::{DELTA_X, DELTA_Y, DELTA_Z, ROTATE_DELTA, X, X_ROT, Y, Y_ROT, Z};
        let all = X | Y | Z | Y_ROT | X_ROT | DELTA_X | DELTA_Y | DELTA_Z | ROTATE_DELTA;
        self.teleport_relative(i, Some(to), all);
    }

    /// Refuses guest `i`'s crossing of passage `id`: the client is told `why`, then put back
    /// where the server has the player, as Immersive Portals does (`teleportEntityGeneral`
    /// to its own position). The flying ticks go on.
    pub fn refuse_passage(&mut self, i: usize, id: u64, why: Refusal) {
        self.tell_refusal(i, id, why);
        self.resend_teleport(i, None);
    }

    fn tell_refusal(&self, i: usize, id: u64, why: Refusal) {
        let reason = refusal_code(why);
        self.guests[i].send_passage_sync(&Message::Refused { id, reason });
    }
}
