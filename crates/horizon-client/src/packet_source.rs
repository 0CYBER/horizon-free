//! Where the game reads a replay's packets from. A replay is a stream of clientbound packets
//! on a virtual timeline; the engine plays it through this trait and does not know the file
//! format behind it (Studio's `.mcpr` driver, `studio/replay.rs`, implements it).
use std::{any::Any, io, time::Duration};

/// A raw timestamped clientbound network packet from a replay stream.
#[derive(Clone, Debug, PartialEq)]
pub struct TimestampedPacket {
    /// Milliseconds since recording start.
    pub timestamp_ms: u32,
    /// Protocol packet ID.
    pub id: i32,
    /// Unframed packet payload bytes (excluding the VarInt packet ID).
    pub payload: Vec<u8>,
}

/// A replay being played: its timeline, its speed and the packets that come due.
pub trait PacketSource {
    /// Current timeline position in milliseconds.
    fn position_ms(&self) -> u32;
    /// Total recording duration in milliseconds.
    fn duration_ms(&self) -> u32;
    /// Total number of indexed packets in the recording stream.
    fn packet_count(&self) -> usize;
    /// Current playback progress as a fraction from 0.0 to 1.0.
    fn progress(&self) -> f32;
    /// Current playback speed multiplier (e.g. 0.25, 1.0, 2.0).
    fn speed(&self) -> f64;
    /// Set the playback speed multiplier.
    fn set_speed(&mut self, speed: f64);
    /// Whether playback is paused.
    fn is_paused(&self) -> bool;
    /// Set the paused state.
    fn set_paused(&mut self, paused: bool);
    /// Toggle the paused state.
    fn toggle_pause(&mut self);
    /// Set whether the replay loops when it reaches the end.
    fn set_looping(&mut self, looping: bool);
    /// Whether playback reached the end of the recording.
    fn is_finished(&self) -> bool;
    /// Advance virtual time by `delta` and collect all packets scheduled up to the new time.
    fn advance(&mut self, delta: Duration) -> io::Result<Vec<TimestampedPacket>>;
    /// Seek directly to a timestamp in milliseconds.
    fn seek(&mut self, target_ms: u32) -> io::Result<()>;
    /// Seek from the beginning and return all packets needed to rebuild the state at
    /// `target_ms`.
    fn seek_and_collect(&mut self, target_ms: u32) -> io::Result<Vec<TimestampedPacket>>;
    /// Move forward from the current position and return the packets crossed.
    fn seek_forward(&mut self, target_ms: u32) -> io::Result<Vec<TimestampedPacket>>;
    /// Rewind playback to the beginning.
    fn restart(&mut self) -> io::Result<()>;
    /// The source itself, for its owner's extras the engine has no name for (Studio's markers
    /// and camera path).
    fn as_any(&self) -> &dyn Any;
    /// As [`PacketSource::as_any`], mutably.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
