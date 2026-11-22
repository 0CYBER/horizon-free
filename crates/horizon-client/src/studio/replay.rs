//! Studio Replay Playback Driver
//!
//! Feeds recorded `.mcpr` packet streams into the client's packet processing
//! loop at user-controlled playback speeds (slow-motion, 1x, fast-forward, pause).

use super::mcpr::{Marker, McprReader, McprWriter, Metadata, TimestampedPacket};
use crate::packet_source::PacketSource;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

/// Playback controller driving recorded packets according to a virtual timeline.
pub struct ReplayDriver {
    pub metadata: Metadata,
    reader: McprReader,
    path: PathBuf,
    /// Where the recording ends: its recorded duration, or its last packet when that is later,
    /// so no packet is ever past the end. The clock never goes past it.
    end_ms: u32,
    /// The Login's time: the recording's opening snapshot is every packet up to it, applied
    /// whenever the world is built from the start, wherever the clock is.
    start_ms: u32,
    cursor_ms: f64,
    speed: f64,
    paused: bool,
    looping: bool,
    finished: bool,
    next_packet_cache: Option<TimestampedPacket>,
    /// A packet read failed since the stream was last read from a seek: the packets taken with
    /// it may be lost, so the stream no longer reads on from the clock (`seek_forward`
    /// refuses) until a seek reads it again.
    broken: bool,
    /// The next packet read fails, as a failing archive's would.
    #[cfg(test)]
    fail_next_read: bool,
    /// The archive the editor's last save wrote, still open (`take_written`).
    written: Option<File>,
}

impl ReplayDriver {
    /// Open an `.mcpr` recording archive for replay playback.
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::from_file(std::fs::File::open(path)?, path)
    }

    /// As [`ReplayDriver::open`], from the archive already open as `file`; `path` is only where
    /// the editor later saves its markers and camera path.
    pub fn from_file(file: File, path: &Path) -> io::Result<Self> {
        Self::from_file_cancellable(file, path, &AtomicBool::new(false))
    }

    /// As [`ReplayDriver::from_file`], giving up with `Interrupted` soon after `cancel` is set
    /// ([`McprReader::from_file_cancellable`]).
    pub fn from_file_cancellable(file: File, path: &Path, cancel: &AtomicBool) -> io::Result<Self> {
        let mut reader = McprReader::from_file_cancellable(file, cancel)?;
        validate_metadata(&reader.metadata)?;
        let metadata = reader.metadata.clone();
        let next_packet_cache = reader.next_packet()?;
        let first = next_packet_cache
            .as_ref()
            .ok_or_else(|| invalid_replay("recording contains no play-state packets"))?;
        validate_play_stream_start(first)?;
        let start_ms = first.timestamp_ms;
        let end_ms = metadata.duration.max(reader.last_timestamp_ms());

        Ok(Self {
            metadata,
            reader,
            path: path.to_path_buf(),
            end_ms,
            start_ms,
            cursor_ms: 0.0,
            speed: 1.0,
            paused: false,
            looping: false,
            finished: false,
            next_packet_cache,
            broken: false,
            #[cfg(test)]
            fail_next_read: false,
            written: None,
        })
    }

    /// The archive the editor's last save (`add_marker`, `save_camera_path`) wrote, still open,
    /// once: what is at [`ReplayDriver::path`] now unless something replaced it since.
    pub fn take_written(&mut self) -> Option<File> {
        self.written.take()
    }

    /// The next packet into the cache; a failure marks the stream broken.
    fn read_next(&mut self) -> io::Result<()> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_read) {
            self.broken = true;
            return Err(io::Error::other("injected read failure"));
        }
        match self.reader.next_packet() {
            Ok(packet) => {
                self.next_packet_cache = packet;
                Ok(())
            }
            Err(error) => {
                self.broken = true;
                Err(error)
            }
        }
    }

    /// The recording's opening snapshot, the Login and every packet with its time, still to be
    /// handed out: what the replay's first frame is set up from, even when the Login is
    /// recorded after 0 ms. The clock doesn't move.
    pub fn take_opening(&mut self) -> io::Result<Vec<TimestampedPacket>> {
        let mut packets = Vec::new();
        while self
            .next_packet_cache
            .as_ref()
            .is_some_and(|packet| packet.timestamp_ms <= self.start_ms)
        {
            packets.push(self.next_packet_cache.take().expect("checked above"));
            self.read_next()?;
        }
        Ok(packets)
    }

    /// The archive this replay reads, where the editor saves its markers and camera path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The editor's camera path saved alongside the packet stream, as the
    /// editor wrote it, if present.
    pub fn camera_path(&self) -> Option<&[u8]> {
        self.reader.camera_path.as_deref()
    }

    /// Markers saved alongside the packet stream, in timeline order.
    pub fn markers(&self) -> &[Marker] {
        &self.reader.markers
    }

    /// Add a bookmark at the current playhead and persist it in `markers.json`.
    pub fn add_marker(&mut self, label: String) -> io::Result<Marker> {
        let marker = Marker {
            time_ms: self.position_ms(),
            label,
        };
        let mut markers = self.reader.markers.clone();
        markers.push(marker.clone());
        markers.sort_by_key(|entry| entry.time_ms);
        self.written = Some(McprWriter::replace_markers(&self.path, &markers)?);
        self.reader.markers = markers;
        Ok(marker)
    }

    /// Save the editor's camera path in the recording archive; `None`
    /// removes it.
    pub fn save_camera_path(&mut self, json: Option<Vec<u8>>) -> io::Result<()> {
        self.written = Some(McprWriter::replace_camera_path(
            &self.path,
            json.as_deref(),
        )?);
        self.reader.camera_path = json;
        Ok(())
    }

    /// Exact playhead for fixed-clock playback.
    pub fn position_ms_f64(&self) -> f64 {
        self.cursor_ms
    }

    /// Deliver packets up to an absolute recording time independently of playback speed,
    /// pause and looping, which the export clock handles.
    pub fn advance_to(&mut self, cursor_ms: f64) -> io::Result<Vec<TimestampedPacket>> {
        if !cursor_ms.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "non-finite replay time",
            ));
        }
        let cursor_ms = cursor_ms.clamp(self.cursor_ms, f64::from(self.end_ms));
        let packets = self.seek_forward(cursor_ms as u32)?;
        self.cursor_ms = cursor_ms;
        Ok(packets)
    }
}

impl PacketSource for ReplayDriver {
    fn position_ms(&self) -> u32 {
        (self.cursor_ms as u32).min(self.end_ms)
    }

    fn duration_ms(&self) -> u32 {
        self.end_ms
    }

    fn packet_count(&self) -> usize {
        self.reader.total_packets()
    }

    fn progress(&self) -> f32 {
        if self.end_ms == 0 {
            0.0
        } else {
            (self.cursor_ms / f64::from(self.end_ms)).clamp(0.0, 1.0) as f32
        }
    }

    fn speed(&self) -> f64 {
        self.speed
    }

    /// A speed that isn't a number changes nothing: `clamp` would keep a NaN.
    fn set_speed(&mut self, speed: f64) {
        if !speed.is_nan() {
            self.speed = speed.clamp(0.01, 20.0);
        }
    }

    fn is_paused(&self) -> bool {
        self.paused
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    fn is_finished(&self) -> bool {
        self.finished
    }

    fn advance(&mut self, delta: Duration) -> io::Result<Vec<TimestampedPacket>> {
        // An ended clock still hands out what a seek left pending at the end (a zero-length
        // recording's snapshot after `restart`).
        if self.paused || (self.finished && self.next_packet_cache.is_none()) {
            return Ok(Vec::new());
        }

        let delta_ms = delta.as_secs_f64() * 1000.0 * self.speed;
        self.cursor_ms = (self.cursor_ms + delta_ms).min(f64::from(self.end_ms));

        let mut ready = Vec::new();
        let target_ms = self.cursor_ms as u32;
        while self
            .next_packet_cache
            .as_ref()
            .is_some_and(|pkt| pkt.timestamp_ms <= target_ms)
        {
            ready.push(self.next_packet_cache.take().expect("checked above"));
            self.read_next()?;
        }
        // The end is the clock reaching `end_ms`; every packet is at or before it, so all of
        // them have been handed out by then.
        if target_ms >= self.end_ms {
            if self.looping {
                self.seek(0)?;
            } else {
                self.finished = true;
            }
        }

        Ok(ready)
    }

    fn seek(&mut self, target_ms: u32) -> io::Result<()> {
        let clamped_ms = target_ms.min(self.end_ms);
        self.cursor_ms = clamped_ms as f64;
        self.finished = clamped_ms >= self.end_ms;

        self.reader.seek_to_timestamp(clamped_ms)?;
        self.read_next()?;
        self.broken = false;
        Ok(())
    }

    fn seek_and_collect(&mut self, target_ms: u32) -> io::Result<Vec<TimestampedPacket>> {
        let target_ms = target_ms.min(self.end_ms);
        self.reader.seek_to_timestamp(0)?;
        self.read_next()?;

        // A world rebuilt from the start always gets the opening snapshot.
        let through_ms = target_ms.max(self.start_ms);
        let mut packets = Vec::new();
        while self
            .next_packet_cache
            .as_ref()
            .is_some_and(|packet| packet.timestamp_ms <= through_ms)
        {
            packets.push(self.next_packet_cache.take().expect("checked above"));
            self.read_next()?;
        }

        self.cursor_ms = target_ms as f64;
        self.finished = target_ms >= self.end_ms;
        self.broken = false;
        Ok(packets)
    }

    fn seek_forward(&mut self, target_ms: u32) -> io::Result<Vec<TimestampedPacket>> {
        let target_ms = target_ms.min(self.end_ms);
        // Against the whole millisecond, not the fractional cursor: every packet up to it has
        // been handed out, so a seek to the position shown is forward too.
        if target_ms < self.position_ms() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot seek backwards without rebuilding replay state",
            ));
        }
        if self.broken {
            return Err(io::Error::other(
                "a packet read failed: the replay has to be rebuilt to seek",
            ));
        }

        let mut packets = Vec::new();
        while self
            .next_packet_cache
            .as_ref()
            .is_some_and(|packet| packet.timestamp_ms <= target_ms)
        {
            packets.push(self.next_packet_cache.take().expect("checked above"));
            self.read_next()?;
        }

        self.cursor_ms = target_ms as f64;
        self.finished = target_ms >= self.end_ms;
        Ok(packets)
    }

    fn restart(&mut self) -> io::Result<()> {
        self.seek(0)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn invalid_replay(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn validate_metadata(metadata: &Metadata) -> io::Result<()> {
    if metadata.file_format != "MCPR" {
        return Err(invalid_replay(format!(
            "unsupported replay file format {:?}",
            metadata.file_format
        )));
    }
    if metadata.file_format_version != 14 {
        return Err(invalid_replay(format!(
            "unsupported MCPR format version {}; this build supports 14",
            metadata.file_format_version
        )));
    }
    match metadata.protocol {
        Some(protocol) if protocol == horizon_protocol::PROTOCOL_VERSION => {}
        Some(protocol) => {
            return Err(invalid_replay(format!(
                "unsupported Minecraft protocol {protocol}; this build supports {}",
                horizon_protocol::PROTOCOL_VERSION
            )));
        }
        None => {
            return Err(invalid_replay(
                "replay does not declare a Minecraft protocol version",
            ));
        }
    }
    Ok(())
}

/// Playback uses the Play packet table, so refuse streams that do not start
/// with the Play login snapshot before interpreting later packet IDs.
fn validate_play_stream_start(first: &TimestampedPacket) -> io::Result<()> {
    let packet = horizon_protocol::packets::play::Clientbound::decode(first.id, &first.payload)
        .map_err(|error| invalid_replay(format!("invalid initial play packet: {error}")))?;
    if !matches!(
        packet,
        horizon_protocol::packets::play::Clientbound::Login(_)
    ) {
        return Err(invalid_replay(
            "recording does not start with the Play Login packet",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::mcpr::McprWriter;
    use std::fs;

    fn play_login_packet() -> (i32, Vec<u8>) {
        use horizon_protocol::{buf::Reader, packets::play};

        let encoded = play::Clientbound::Login(play::Login {
            player_id: 1,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: false,
            enforces_secure_chat: false,
        })
        .encode();
        let mut reader = Reader::new(&encoded);
        let id = reader.varint().unwrap();
        (id, reader.rest().to_vec())
    }

    #[test]
    fn playback_refuses_unknown_format_or_protocol_metadata() {
        let mut metadata = Metadata::default();
        assert!(validate_metadata(&metadata).is_ok());

        metadata.file_format = "OTHER".into();
        assert_eq!(
            validate_metadata(&metadata).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        metadata = Metadata::default();
        metadata.file_format_version = 13;
        assert_eq!(
            validate_metadata(&metadata).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        metadata = Metadata::default();
        metadata.protocol = None;
        assert_eq!(
            validate_metadata(&metadata).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        metadata.protocol = Some(horizon_protocol::PROTOCOL_VERSION + 1);
        assert_eq!(
            validate_metadata(&metadata).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn playback_requires_a_play_login_phase_start() {
        let (id, payload) = play_login_packet();
        assert!(validate_play_stream_start(&TimestampedPacket::new(0, id, payload)).is_ok());

        use horizon_protocol::packets::play;
        let encoded =
            play::Clientbound::SetChunkCacheCenter(play::SetChunkCacheCenter { x: 0, z: 0 })
                .encode();
        let mut reader = horizon_protocol::buf::Reader::new(&encoded);
        let id = reader.varint().unwrap();
        assert_eq!(
            validate_play_stream_start(&TimestampedPacket::new(0, id, reader.rest().to_vec()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn test_replay_driver_advance_and_speed() {
        let temp_dir =
            std::env::temp_dir().join(format!("replay_driver_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let mcpr_path = temp_dir.join("driver_test.mcpr");

        let meta = Metadata {
            duration: 1000,
            ..Default::default()
        };
        let mut writer = McprWriter::new(mcpr_path.clone(), meta).unwrap();
        let (login_id, login_payload) = play_login_packet();
        writer.write_packet(0, login_id, &login_payload).unwrap();
        writer.write_packet(0, 0x01, &[1]).unwrap();
        writer.write_packet(100, 0x02, &[2]).unwrap();
        writer.write_packet(200, 0x03, &[3]).unwrap();
        writer.write_packet(500, 0x04, &[4]).unwrap();
        writer.finish(1000).unwrap();

        let mut driver = ReplayDriver::open(&mcpr_path).unwrap();
        assert_eq!(driver.position_ms(), 0);
        assert_eq!(driver.duration_ms(), 1000);

        // Advance 50ms at 1.0x -> receives packet at 0ms
        let batch1 = driver.advance(Duration::from_millis(50)).unwrap();
        assert_eq!(batch1.len(), 2);
        assert_eq!(batch1[0].id, login_id);
        assert_eq!(batch1[1].id, 0x01);

        // Advance another 100ms (now 150ms) -> receives packet at 100ms
        let batch2 = driver.advance(Duration::from_millis(100)).unwrap();
        assert_eq!(batch2.len(), 1);
        assert_eq!(batch2[0].id, 0x02);

        // Paused -> advance produces nothing
        driver.set_paused(true);
        let batch_paused = driver.advance(Duration::from_millis(200)).unwrap();
        assert!(batch_paused.is_empty());
        driver.set_paused(false);

        // Speed 2.0x -> advance 100ms real time advances 200ms virtual time (150ms -> 350ms)
        driver.set_speed(2.0);
        let batch3 = driver.advance(Duration::from_millis(100)).unwrap();
        assert_eq!(batch3.len(), 1);
        assert_eq!(batch3[0].id, 0x03);

        // Seek back to 0
        driver.seek(0).unwrap();
        assert_eq!(driver.position_ms(), 0);

        let _ = fs::remove_file(mcpr_path);
        let _ = fs::remove_dir(temp_dir);
    }

    #[test]
    fn seek_reconstructs_packets_and_forward_seek_skips_consumed_packets() {
        let temp_dir =
            std::env::temp_dir().join(format!("replay_driver_seek_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let mcpr_path = temp_dir.join("seek_test.mcpr");
        let meta = Metadata {
            duration: 1000,
            ..Default::default()
        };
        let mut writer = McprWriter::new(mcpr_path.clone(), meta).unwrap();
        let (login_id, login_payload) = play_login_packet();
        writer.write_packet(0, login_id, &login_payload).unwrap();
        for (timestamp, id) in [(0, 1), (100, 2), (200, 3), (500, 4)] {
            writer.write_packet(timestamp, id, &[id as u8]).unwrap();
        }
        writer.finish(1000).unwrap();

        let mut driver = ReplayDriver::open(&mcpr_path).unwrap();
        let at_200 = driver.seek_and_collect(200).unwrap();
        assert_eq!(
            at_200.iter().map(|packet| packet.id).collect::<Vec<_>>(),
            [login_id, 1, 2, 3]
        );
        assert_eq!(driver.position_ms(), 200);

        let forward = driver.seek_forward(500).unwrap();
        assert_eq!(
            forward.iter().map(|packet| packet.id).collect::<Vec<_>>(),
            [4]
        );
        assert_eq!(driver.position_ms(), 500);

        // A clock between two whole milliseconds: a seek to the one shown is forward, and
        // hands out nothing again.
        driver.set_speed(0.5);
        assert!(
            driver
                .advance(Duration::from_millis(33))
                .unwrap()
                .is_empty()
        );
        assert_eq!(driver.position_ms(), 516);
        assert!(driver.seek_forward(516).unwrap().is_empty());
        assert_eq!(driver.position_ms(), 516);
        assert_eq!(
            driver.seek_forward(515).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        let _ = fs::remove_file(mcpr_path);
        let _ = fs::remove_dir(temp_dir);
    }

    /// A packet read that fails while playing loses the packets taken with it: a later forward
    /// seek refuses (the caller rebuilds) rather than skip them, until a seek reads the stream
    /// again.
    #[test]
    fn a_failed_read_makes_forward_seeks_refuse_until_the_stream_is_read_again() {
        let temp_dir =
            std::env::temp_dir().join(format!("replay_driver_broken_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let mcpr_path = temp_dir.join("broken_test.mcpr");
        let meta = Metadata {
            duration: 1000,
            ..Default::default()
        };
        let mut writer = McprWriter::new(mcpr_path.clone(), meta).unwrap();
        let (login_id, login_payload) = play_login_packet();
        writer.write_packet(0, login_id, &login_payload).unwrap();
        for (timestamp, id) in [(200, 2), (300, 3), (500, 4)] {
            writer.write_packet(timestamp, id, &[id as u8]).unwrap();
        }
        writer.finish(1000).unwrap();

        let mut driver = ReplayDriver::open(&mcpr_path).unwrap();
        assert_eq!(driver.advance(Duration::from_millis(100)).unwrap().len(), 1);
        // Reading on after the packet at 200 fails: it is taken, and lost with the batch.
        driver.fail_next_read = true;
        assert!(driver.advance(Duration::from_millis(150)).is_err());
        assert!(driver.seek_forward(400).is_err(), "refused, not skipped");
        assert!(driver.seek_forward(600).is_err());

        let rebuilt = driver.seek_and_collect(400).unwrap();
        assert_eq!(
            rebuilt.iter().map(|packet| packet.id).collect::<Vec<_>>(),
            [login_id, 2, 3]
        );
        let forward = driver.seek_forward(600).unwrap();
        assert_eq!(
            forward.iter().map(|packet| packet.id).collect::<Vec<_>>(),
            [4]
        );

        // A failed forward seek breaks it too; a plain seek reads it again.
        driver.seek(0).unwrap();
        driver.fail_next_read = true;
        assert!(driver.seek_forward(250).is_err());
        assert!(driver.seek_forward(900).is_err());
        driver.seek(0).unwrap();
        assert_eq!(
            driver.seek_forward(250).unwrap().len(),
            2,
            "the Login and the packet at 200"
        );

        let _ = fs::remove_file(mcpr_path);
        let _ = fs::remove_dir(temp_dir);
    }
}
