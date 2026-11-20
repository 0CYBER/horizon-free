//! What a server's diagnostics read: the traffic of a player's connection and the time mobs take.

/// One packet id's traffic in one direction of a player's connection, since the counts were last
/// taken (`player_take_packet_counts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PacketCount {
    /// Sent by the server (clientbound); false for one the client sent.
    pub outbound: bool,
    /// The play packet id as the server sent or read it.
    pub id: i32,
    /// How many there were.
    pub packets: u64,
    /// Their bytes of id and payload, before compression and encryption.
    pub bytes: u64,
}

impl PacketCount {
    /// A count of `packets` packets of `bytes` in all, with id `id`.
    pub fn new(outbound: bool, id: i32, packets: u64, bytes: u64) -> Self {
        Self {
            outbound,
            id,
            packets,
            bytes,
        }
    }
}

/// The time one mob type's own ticks took in one level while sampled (`take_mob_costs`); a
/// mob's riders tick inside its tick, so they count for it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct MobTickCost {
    /// The level (`minecraft:overworld`).
    pub dimension: String,
    /// The mob type (`minecraft:zombie`).
    pub kind: String,
    /// The nanoseconds their ticks took.
    pub nanos: u64,
    /// How many mob ticks were timed.
    pub mob_ticks: u64,
}

impl MobTickCost {
    /// `mob_ticks` ticks of `kind` in `dimension` that took `nanos`.
    pub fn new(dimension: String, kind: String, nanos: u64, mob_ticks: u64) -> Self {
        Self {
            dimension,
            kind,
            nanos,
            mob_ticks,
        }
    }
}
